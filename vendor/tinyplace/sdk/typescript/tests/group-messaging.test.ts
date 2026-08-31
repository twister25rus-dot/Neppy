import { describe, expect, it } from "vitest";
import {
  GroupKeyManager,
  buildGroupEnvelope,
  decodeGroupBody,
  encodeGroupKeyDistribution,
  parseGroupKeyDistribution,
  parseSenderKeyId,
} from "../src/index.js";

describe("group messaging helpers", () => {
  it("round-trips a sender-key id", () => {
    const id = "group-1:ALICEPUB:epoch:3";
    expect(parseSenderKeyId(id)).toEqual({
      groupId: "group-1",
      sender: "ALICEPUB",
      epoch: 3,
    });
    expect(parseSenderKeyId("not-a-key")).toBeNull();
  });

  it("round-trips a group key distribution control payload", () => {
    const distribution = {
      chainKey: "Y2hhaW4=",
      iteration: 0,
      signaturePublicKey: "c2ln",
    };
    const encoded = encodeGroupKeyDistribution(
      "group-1",
      "ALICE",
      2,
      distribution,
    );
    const parsed = parseGroupKeyDistribution(encoded);
    expect(parsed?.groupId).toBe("group-1");
    expect(parsed?.epoch).toBe(2);
    expect(parsed?.distribution).toEqual(distribution);
    expect(parseGroupKeyDistribution("hello plaintext")).toBeNull();
  });

  it("encrypts and decrypts a real group message end-to-end via sender keys", async () => {
    const groupId = "group-1";
    const sender = "ALICE_AGENT";
    const epoch = 0;
    const members = [sender, "BOB_AGENT"];

    // Alice (sender) creates her sending key and hands out the distribution.
    const alice = new GroupKeyManager();
    const senderKey = alice.ensureOwn(groupId, epoch);
    expect(alice.pendingDistribution(groupId, epoch, members, sender)).toEqual([
      "BOB_AGENT",
    ]);
    const distributionPayload = encodeGroupKeyDistribution(
      groupId,
      sender,
      epoch,
      senderKey.distribution(),
    );

    // Bob installs the distribution he received over the DM channel.
    const bob = new GroupKeyManager();
    bob.installReceiver(parseGroupKeyDistribution(distributionPayload)!);

    // Alice encrypts + fans out; the wire envelope is opaque binary, not the text.
    const encrypted = await senderKey.encrypt(
      new TextEncoder().encode("gm team"),
    );
    const envelope = buildGroupEnvelope(
      "g1",
      groupId,
      sender,
      epoch,
      encrypted,
    );
    expect(envelope.body).not.toContain("gm team");
    expect(envelope.signal?.senderKeyId).toBe(`${groupId}:${sender}:epoch:${epoch}`);

    // Bob reconstructs and decrypts using the receiver for (group, sender, epoch).
    const parsedId = parseSenderKeyId(envelope.signal!.senderKeyId!)!;
    const receiver = bob.getReceiver(
      parsedId.groupId,
      parsedId.sender,
      parsedId.epoch,
    )!;
    const message = decodeGroupBody(
      envelope.body,
      envelope.signal!.senderKeyIteration!,
    )!;
    const plaintext = await receiver.decrypt(message);
    expect(new TextDecoder().decode(plaintext)).toBe("gm team");
  });
});
