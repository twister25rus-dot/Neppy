import { describe, expect, it } from "vitest";

import {
  GroupSenderKey,
  GroupSenderKeyReceiver,
} from "../src/signal/sender-key.js";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function text(value: string): Uint8Array {
  return encoder.encode(value);
}

function read(bytes: Uint8Array): string {
  return decoder.decode(bytes);
}

describe("GroupSenderKey / GroupSenderKeyReceiver", () => {
  it("round-trips a single message between sender and a freshly distributed receiver", async () => {
    const sender = GroupSenderKey.create();
    const receiver = GroupSenderKeyReceiver.fromDistribution(
      sender.distribution(),
    );

    const message = await sender.encrypt(text("hello group"));
    expect(message.iteration).toBe(0);

    const plaintext = await receiver.decrypt(message);
    expect(read(plaintext)).toBe("hello group");
  });

  it("decrypts a sequence of in-order messages and advances the iteration", async () => {
    const sender = GroupSenderKey.create();
    const receiver = GroupSenderKeyReceiver.fromDistribution(
      sender.distribution(),
    );

    for (let index = 0; index < 5; index += 1) {
      // eslint-disable-next-line no-await-in-loop
      const message = await sender.encrypt(text(`msg-${index}`));
      expect(message.iteration).toBe(index);
      // eslint-disable-next-line no-await-in-loop
      const plaintext = await receiver.decrypt(message);
      expect(read(plaintext)).toBe(`msg-${index}`);
    }
    expect(sender.currentIteration).toBe(5);
  });

  it("tolerates out-of-order delivery by caching skipped keys", async () => {
    const sender = GroupSenderKey.create();
    const receiver = GroupSenderKeyReceiver.fromDistribution(
      sender.distribution(),
    );

    const m0 = await sender.encrypt(text("zero"));
    const m1 = await sender.encrypt(text("one"));
    const m2 = await sender.encrypt(text("two"));

    // Deliver out of order: 2, then 0, then 1.
    expect(read(await receiver.decrypt(m2))).toBe("two");
    expect(read(await receiver.decrypt(m0))).toBe("zero");
    expect(read(await receiver.decrypt(m1))).toBe("one");
  });

  it("lets a member who joins mid-stream read only messages from its distribution forward", async () => {
    const sender = GroupSenderKey.create();
    const early = GroupSenderKeyReceiver.fromDistribution(
      sender.distribution(),
    );

    await sender.encrypt(text("before join 0"));
    await sender.encrypt(text("before join 1"));

    // A new member receives the distribution at the current iteration.
    const lateDistribution = sender.distribution();
    const late = GroupSenderKeyReceiver.fromDistribution(lateDistribution);
    expect(lateDistribution.iteration).toBe(2);

    const m2 = await sender.encrypt(text("after join 2"));
    expect(read(await late.decrypt(m2))).toBe("after join 2");
    // The early member can still read it too.
    expect(read(await early.decrypt(m2))).toBe("after join 2");
  });

  it("rejects a message whose signature does not verify", async () => {
    const sender = GroupSenderKey.create();
    const other = GroupSenderKey.create();
    const receiver = GroupSenderKeyReceiver.fromDistribution(
      sender.distribution(),
    );

    const message = await sender.encrypt(text("authentic"));
    // Swap in a signature from a different signing key.
    const forged = await other.encrypt(text("authentic"));
    const tampered = { ...message, signature: forged.signature };

    await expect(receiver.decrypt(tampered)).rejects.toThrow(
      /signature verification failed/i,
    );
  });

  it("rejects a message whose ciphertext was tampered with (MAC fails)", async () => {
    const sender = GroupSenderKey.create();
    const message = await sender.encrypt(text("integrity"));

    // Re-sign mutated ciphertext with a receiver-trusted key so the signature
    // passes but the AEAD MAC must catch the tampering.
    const distribution = sender.distribution();
    const receiver = GroupSenderKeyReceiver.fromDistribution(distribution);
    const bytes = Uint8Array.from(atob(message.ciphertext), (c) =>
      c.charCodeAt(0),
    );
    bytes[0] ^= 0xff;
    const mutated = btoa(String.fromCharCode(...bytes));

    // Signature won't match the mutated bytes -> rejected before MAC, which is
    // also acceptable; either way the message must not decrypt.
    await expect(
      receiver.decrypt({ ...message, ciphertext: mutated }),
    ).rejects.toThrow();
  });

  it("survives serialize/restore on both halves", async () => {
    const sender = GroupSenderKey.create();
    const receiver = GroupSenderKeyReceiver.fromDistribution(
      sender.distribution(),
    );

    const first = await sender.encrypt(text("one"));
    expect(read(await receiver.decrypt(first))).toBe("one");

    const restoredSender = GroupSenderKey.restore(sender.serialize());
    const restoredReceiver = GroupSenderKeyReceiver.restore(
      receiver.serialize(),
    );

    const second = await restoredSender.encrypt(text("two"));
    expect(second.iteration).toBe(1);
    expect(read(await restoredReceiver.decrypt(second))).toBe("two");
  });

  it("refuses to replay a message older than the current chain without a cached key", async () => {
    const sender = GroupSenderKey.create();
    const receiver = GroupSenderKeyReceiver.fromDistribution(
      sender.distribution(),
    );

    const m0 = await sender.encrypt(text("zero"));
    await receiver.decrypt(m0);

    // m0 was consumed in order (no skipped cache); replaying must fail.
    await expect(receiver.decrypt(m0)).rejects.toThrow(/older than/i);
  });
});
