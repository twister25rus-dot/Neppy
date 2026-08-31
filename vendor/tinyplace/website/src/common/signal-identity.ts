import { LocalSigner, type X25519KeyPair } from "@tinyhumansai/tinyplace";

import {
	IndexedDbSessionStore,
	loadStoredSeed,
	openSignalDatabase,
	storeSeed,
} from "@src/common/signal-store";

type SignMessageFunction = (message: Uint8Array) => Promise<Uint8Array>;

/**
 * Fixed message the wallet signs to derive the end-to-end encryption identity.
 * The signature is deterministic per wallet, so the same wallet always recovers
 * the same encryption keys. Bumping the version invalidates derived keys.
 */
export const ENCRYPTION_IDENTITY_MESSAGE =
	"tiny.place encryption identity v1\n\n" +
	"Sign this message to derive your end-to-end encryption keys. " +
	"This is free, off-chain, and does not authorize any transaction.";

/**
 * The Signal/encryption identity for the current wallet. This is intentionally
 * separate from the wallet itself: the wallet still signs API auth and payments,
 * while this derived key powers Signal Protocol (X3DH + Double Ratchet). It is
 * derived deterministically from a wallet signature, so it is recoverable.
 */
export interface SignalIdentity {
	/** Ed25519 signer for the encryption identity (signs pre-keys, decrypts). */
	signer: LocalSigner;
	/** Long-term X25519 identity key pair used in X3DH. */
	identityKeyPair: X25519KeyPair;
	/** Raw Ed25519 identity public key (published in the agent's key bundle). */
	identityPublicKey: Uint8Array;
	/** Persistent IndexedDB-backed Signal session store. */
	store: IndexedDbSessionStore;
}

async function deriveSeedFromSignature(
	signature: Uint8Array
): Promise<Uint8Array> {
	// Copy into a fresh ArrayBuffer so the type is a plain BufferSource (avoids
	// the SharedArrayBuffer ambiguity in Uint8Array<ArrayBufferLike>).
	const buffer = new ArrayBuffer(signature.byteLength);
	new Uint8Array(buffer).set(signature);
	const digest = await globalThis.crypto.subtle.digest("SHA-256", buffer);
	return new Uint8Array(digest);
}

/**
 * Loads the wallet's encryption identity, deriving and persisting it on first
 * use. On the first call for a wallet the user is prompted to sign
 * {@link ENCRYPTION_IDENTITY_MESSAGE}; afterwards the seed is read from
 * IndexedDB and no prompt is shown.
 *
 * @param walletAgentId - The wallet-derived agent id that scopes this identity.
 * @param signMessage - The wallet's message-signing function.
 * @returns The resolved Signal identity (signer, keys, and session store).
 */
export async function loadOrCreateSignalIdentity(
	walletAgentId: string,
	signMessage: SignMessageFunction
): Promise<SignalIdentity> {
	const db = await openSignalDatabase();

	let seed = await loadStoredSeed(db, walletAgentId);
	if (!seed) {
		const message = new TextEncoder().encode(ENCRYPTION_IDENTITY_MESSAGE);
		const signature = await signMessage(message);
		seed = await deriveSeedFromSignature(signature);
		await storeSeed(db, walletAgentId, seed);
	}

	const signer = await LocalSigner.fromSeed(seed);
	const identityKeyPair = await signer.getX25519KeyPair();
	const store = new IndexedDbSessionStore(db, walletAgentId, identityKeyPair);

	return {
		signer,
		identityKeyPair,
		identityPublicKey: signer.publicKey,
		store,
	};
}

/**
 * Whether the given wallet already has a persisted encryption identity (so it
 * can be loaded without prompting for a signature).
 *
 * @param walletAgentId - The wallet-derived agent id.
 * @returns True if a seed is already stored for this wallet.
 */
export async function hasSignalIdentity(
	walletAgentId: string
): Promise<boolean> {
	const db = await openSignalDatabase();
	const seed = await loadStoredSeed(db, walletAgentId);
	return seed !== undefined;
}
