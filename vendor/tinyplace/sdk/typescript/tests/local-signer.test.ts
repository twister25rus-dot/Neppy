import { describe, it, expect } from "vitest";
import { LocalSigner } from "../src/index.js";
import { ed25519PubToX25519Pub, toBase64 } from "../src/signal/crypto.js";

const seed = new Uint8Array(32).map((_, i) => (i * 7 + 3) & 0xff);

describe("LocalSigner.fromSeed", () => {
  it("rejects seeds that are not 32 bytes", async () => {
    await expect(LocalSigner.fromSeed(new Uint8Array(16))).rejects.toThrow();
  });

  it("is deterministic — same seed yields the same identity", async () => {
    const a = await LocalSigner.fromSeed(seed);
    const b = await LocalSigner.fromSeed(seed);
    expect(a.agentId).toBe(b.agentId);
    expect(a.agentId).toBe(encodeBase58(a.publicKey));
    expect(a.agentId.startsWith("tiny")).toBe(false);
    expect(a.publicKeyBase64).toBe(b.publicKeyBase64);
  });

  it("different seeds yield different identities", async () => {
    const a = await LocalSigner.fromSeed(seed);
    const other = new Uint8Array(32).fill(9);
    const b = await LocalSigner.fromSeed(other);
    expect(a.publicKeyBase64).not.toBe(b.publicKeyBase64);
  });

  it("produces verifiable Ed25519 signatures", async () => {
    const signer = await LocalSigner.fromSeed(seed);
    const message = new TextEncoder().encode("hello tiny.place");
    const signature = await signer.sign(message);
    const key = await globalThis.crypto.subtle.importKey(
      "raw",
      signer.publicKey,
      { name: "Ed25519" },
      false,
      ["verify"],
    );
    const ok = await globalThis.crypto.subtle.verify(
      "Ed25519",
      key,
      signature,
      message,
    );
    expect(ok).toBe(true);
  });

  it("derives an X25519 keypair consistent with the Ed25519 identity key", async () => {
    const signer = await LocalSigner.fromSeed(seed);
    const x = await signer.getX25519KeyPair();
    // The X25519 public key must equal the Montgomery form of the Ed25519
    // identity key, which is what X3DH uses on the other side.
    expect(toBase64(x.publicKey)).toBe(
      toBase64(ed25519PubToX25519Pub(signer.publicKey)),
    );
  });

  it("recovers from a 64-byte Solana base58 secret key", async () => {
    const original = await LocalSigner.fromSeed(seed);
    const secretKey = new Uint8Array(64);
    secretKey.set(seed, 0);
    secretKey.set(original.publicKey, 32);

    const recovered = await LocalSigner.fromSolanaSecretKey(
      encodeBase58(secretKey),
    );

    expect(recovered.agentId).toBe(encodeBase58(original.publicKey));
    expect(recovered.agentId.startsWith("tiny")).toBe(false);
    expect(recovered.publicKeyBase64).toBe(original.publicKeyBase64);
  });

  it("recovers from a 32-byte Solana seed", async () => {
    const original = await LocalSigner.fromSeed(seed);
    const recovered = await LocalSigner.fromSolanaSecretKey(encodeBase58(seed));

    expect(recovered.agentId).toBe(encodeBase58(original.publicKey));
    expect(recovered.agentId.startsWith("tiny")).toBe(false);
    expect(recovered.publicKeyBase64).toBe(original.publicKeyBase64);
  });

  it("rejects malformed Solana secret keys", async () => {
    const malformed = new Uint8Array(64);
    malformed.set(seed, 0);
    malformed.fill(9, 32);

    await expect(
      LocalSigner.fromSolanaSecretKey(encodeBase58(malformed)),
    ).rejects.toThrow("public key does not match");
    await expect(LocalSigner.fromSolanaSecretKey("0")).rejects.toThrow(
      "Invalid base58 character",
    );
  });
});

const BASE58_ALPHABET =
  "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

function encodeBase58(bytes: Uint8Array): string {
  let value = 0n;
  for (const byte of bytes) {
    value = (value << 8n) + BigInt(byte);
  }

  let encoded = "";
  while (value > 0n) {
    const digit = Number(value % 58n);
    encoded = BASE58_ALPHABET[digit]! + encoded;
    value /= 58n;
  }

  for (const byte of bytes) {
    if (byte !== 0) break;
    encoded = "1" + encoded;
  }

  return encoded || "1";
}
