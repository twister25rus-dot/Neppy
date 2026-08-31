import { ed25519 } from "@noble/curves/ed25519.js";

import {
  decrypt,
  encrypt,
  fromBase64,
  kdfChainKey,
  toBase64,
} from "./crypto.js";

const cryptoRef = globalThis.crypto;

/** Cap on how far a receiver will fast-forward to reach an out-of-order message. */
const MAX_SKIP = 2000;

/** Group sender-key messages carry no extra associated data; the ed25519 signature authenticates the sender. */
const EMPTY_AD = new Uint8Array(0);

/**
 * Public, distributable snapshot of a sender's group key. This is what a sender
 * shares — over an already-secure 1:1 channel — so other members can decrypt its
 * group messages. It exposes the chain key at a single iteration; a receiver
 * initialised from it can read messages from that iteration forward (earlier
 * messages stay secret, by forward secrecy).
 */
export interface SenderKeyDistribution {
  /** Base64 chain key corresponding to `iteration`. */
  chainKey: string;
  /** Message number the chain key corresponds to. */
  iteration: number;
  /** Base64 ed25519 public key used to verify this sender's messages. */
  signaturePublicKey: string;
}

/** A single encrypted group message produced by a {@link GroupSenderKey}. */
export interface SenderKeyMessage {
  /** Message number within the sender's chain. */
  iteration: number;
  /** Base64 AEAD ciphertext (with appended MAC). */
  ciphertext: string;
  /** Base64 ed25519 signature over the raw ciphertext bytes. */
  signature: string;
}

/** Serialised form of a {@link GroupSenderKey} (the sending half) for persistence. */
export interface SenderKeyOwnState {
  chainKey: string;
  iteration: number;
  signaturePrivateKey: string;
  signaturePublicKey: string;
}

/** Serialised form of a {@link GroupSenderKeyReceiver} (the receiving half). */
export interface SenderKeyReceiverState {
  chainKey: string;
  iteration: number;
  signaturePublicKey: string;
  skipped: Record<number, string>;
}

/**
 * The sending half of a Signal Sender Key: a symmetric chain key ratcheted once
 * per message, plus an ed25519 key pair that signs every message so receivers
 * can attribute it to this sender. One instance per (group, sender, membership
 * epoch); a new epoch (membership change) means a fresh key.
 */
export class GroupSenderKey {
  private chainKey: Uint8Array;
  private iteration: number;
  private readonly signaturePrivateKey: Uint8Array;
  private readonly signaturePublicKey: Uint8Array;

  private constructor(
    chainKey: Uint8Array,
    iteration: number,
    signaturePrivateKey: Uint8Array,
    signaturePublicKey: Uint8Array,
  ) {
    this.chainKey = chainKey;
    this.iteration = iteration;
    this.signaturePrivateKey = signaturePrivateKey;
    this.signaturePublicKey = signaturePublicKey;
  }

  /** Generates a brand-new sender key with a random chain key and signing pair. */
  static create(): GroupSenderKey {
    const chainKey = cryptoRef.getRandomValues(new Uint8Array(32));
    const signaturePrivateKey = ed25519.utils.randomSecretKey();
    const signaturePublicKey = ed25519.getPublicKey(signaturePrivateKey);
    return new GroupSenderKey(
      chainKey,
      0,
      signaturePrivateKey,
      signaturePublicKey,
    );
  }

  /** Rebuilds a sender key from {@link serialize} output. */
  static restore(state: SenderKeyOwnState): GroupSenderKey {
    return new GroupSenderKey(
      fromBase64(state.chainKey),
      state.iteration,
      fromBase64(state.signaturePrivateKey),
      fromBase64(state.signaturePublicKey),
    );
  }

  /** The current message number (advances after each {@link encrypt}). */
  get currentIteration(): number {
    return this.iteration;
  }

  /** Persistable snapshot including the private signing key. Keep this secret. */
  serialize(): SenderKeyOwnState {
    return {
      chainKey: toBase64(this.chainKey),
      iteration: this.iteration,
      signaturePrivateKey: toBase64(this.signaturePrivateKey),
      signaturePublicKey: toBase64(this.signaturePublicKey),
    };
  }

  /** Snapshot to hand to other members so they can decrypt from here forward. */
  distribution(): SenderKeyDistribution {
    return {
      chainKey: toBase64(this.chainKey),
      iteration: this.iteration,
      signaturePublicKey: toBase64(this.signaturePublicKey),
    };
  }

  /** Encrypts and signs one group message, ratcheting the chain forward. */
  async encrypt(plaintext: Uint8Array): Promise<SenderKeyMessage> {
    const iteration = this.iteration;
    const { chainKey, messageKey } = kdfChainKey(this.chainKey);
    const ciphertext = await encrypt(messageKey, plaintext, EMPTY_AD);
    const signature = ed25519.sign(ciphertext, this.signaturePrivateKey);
    this.chainKey = chainKey;
    this.iteration = iteration + 1;
    return {
      iteration,
      ciphertext: toBase64(ciphertext),
      signature: toBase64(signature),
    };
  }
}

/**
 * The receiving half of a Sender Key: holds another member's chain key and their
 * signature public key, derived from a {@link SenderKeyDistribution}. Verifies
 * and decrypts that sender's group messages, tolerating out-of-order delivery by
 * caching skipped message keys.
 */
export class GroupSenderKeyReceiver {
  private chainKey: Uint8Array;
  private iteration: number;
  private readonly signaturePublicKey: Uint8Array;
  private readonly skipped: Map<number, Uint8Array>;

  private constructor(
    chainKey: Uint8Array,
    iteration: number,
    signaturePublicKey: Uint8Array,
    skipped: Map<number, Uint8Array>,
  ) {
    this.chainKey = chainKey;
    this.iteration = iteration;
    this.signaturePublicKey = signaturePublicKey;
    this.skipped = skipped;
  }

  /** Initialises a receiver from a sender's distribution snapshot. */
  static fromDistribution(
    distribution: SenderKeyDistribution,
  ): GroupSenderKeyReceiver {
    return new GroupSenderKeyReceiver(
      fromBase64(distribution.chainKey),
      distribution.iteration,
      fromBase64(distribution.signaturePublicKey),
      new Map(),
    );
  }

  /** Rebuilds a receiver from {@link serialize} output. */
  static restore(state: SenderKeyReceiverState): GroupSenderKeyReceiver {
    const skipped = new Map<number, Uint8Array>();
    for (const [iteration, key] of Object.entries(state.skipped)) {
      skipped.set(Number(iteration), fromBase64(key));
    }
    return new GroupSenderKeyReceiver(
      fromBase64(state.chainKey),
      state.iteration,
      fromBase64(state.signaturePublicKey),
      skipped,
    );
  }

  /** Persistable snapshot of the receiver chain and any cached skipped keys. */
  serialize(): SenderKeyReceiverState {
    const skipped: Record<number, string> = {};
    for (const [iteration, key] of this.skipped.entries()) {
      skipped[iteration] = toBase64(key);
    }
    return {
      chainKey: toBase64(this.chainKey),
      iteration: this.iteration,
      signaturePublicKey: toBase64(this.signaturePublicKey),
      skipped,
    };
  }

  /** Verifies the signature, then decrypts the message at its iteration. */
  async decrypt(message: SenderKeyMessage): Promise<Uint8Array> {
    const ciphertext = fromBase64(message.ciphertext);
    const signature = fromBase64(message.signature);
    if (!ed25519.verify(signature, ciphertext, this.signaturePublicKey)) {
      throw new Error("Sender key signature verification failed");
    }
    const messageKey = this.messageKeyFor(message.iteration);
    return decrypt(messageKey, ciphertext, EMPTY_AD);
  }

  /**
   * Returns the message key for `target`, advancing the chain and caching keys
   * for any skipped (not-yet-seen) iterations along the way.
   */
  private messageKeyFor(target: number): Uint8Array {
    const cached = this.skipped.get(target);
    if (cached) {
      this.skipped.delete(target);
      return cached;
    }
    if (target < this.iteration) {
      throw new Error("Sender key message is older than the current chain");
    }
    if (target - this.iteration > MAX_SKIP) {
      throw new Error("Too many skipped sender key messages");
    }
    while (this.iteration < target) {
      const step = kdfChainKey(this.chainKey);
      this.skipped.set(this.iteration, step.messageKey);
      this.chainKey = step.chainKey;
      this.iteration += 1;
    }
    const { chainKey, messageKey } = kdfChainKey(this.chainKey);
    this.chainKey = chainKey;
    this.iteration += 1;
    return messageKey;
  }
}
