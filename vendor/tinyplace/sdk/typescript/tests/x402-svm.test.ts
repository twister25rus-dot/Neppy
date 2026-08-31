import { describe, expect, it } from "vitest";

import {
  SOLANA_USDC_MINT,
  buildExactSvmTransferTransaction,
  deriveAssociatedTokenAddress,
} from "../src/solana.js";

const FEE_PAYER = "EwWqGE4ZFKLofuestmU4LDdK7XM1N4ALgdZccwYugwGd";
const PAY_TO = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";

function base64ToBytes(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

describe("deriveAssociatedTokenAddress", () => {
  // Oracle vector generated with gagliardetto/solana-go (the exact library the
  // backend x402 facilitator uses to derive the destination ATA it verifies):
  //   FindAssociatedTokenAddress(owner, USDC mint) -> FGETo8... (bump 254)
  it("matches the solana-go ATA for a known owner+mint", () => {
    const owner = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
    expect(deriveAssociatedTokenAddress(owner, SOLANA_USDC_MINT)).toBe(
      "FGETo8T8wMcN2wCjav8VK6eh3dLk63evNDPxzLSJra8B",
    );
  });
});

describe("buildExactSvmTransferTransaction", () => {
  const baseOptions = {
    secretKey: new Uint8Array(32).fill(7),
    feePayer: FEE_PAYER,
    payTo: PAY_TO,
    mint: SOLANA_USDC_MINT,
    amount: "1000000",
    decimals: 6,
    recentBlockhash: "11111111111111111111111111111111",
    memo: "pi_test_123",
  };

  it("builds a partially-signed exact-SVM transfer with the spec layout", () => {
    const built = buildExactSvmTransferTransaction(baseOptions);

    // Returned fields: the payer authority is deterministic from the seed, and
    // the destination is the ATA the facilitator verifies (payTo + mint).
    expect(built.from).toBe("GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB");
    expect(built.destinationTokenAccount).toBe(
      deriveAssociatedTokenAddress(PAY_TO, SOLANA_USDC_MINT),
    );
    expect(built.memo).toBe("pi_test_123");

    const bytes = base64ToBytes(built.transaction);
    // 2 signatures: fee payer slot zeroed (facilitator co-signs), authority signed.
    expect(bytes[0]).toBe(2);
    const sig0 = bytes.slice(1, 65);
    const sig1 = bytes.slice(65, 129);
    expect(sig0.every((b) => b === 0)).toBe(true);
    expect(sig1.some((b) => b !== 0)).toBe(true);
    // Message header: 2 required signatures, 1 readonly signed, 4 readonly unsigned.
    expect(Array.from(bytes.slice(129, 132))).toEqual([2, 1, 4]);
    // 8 accounts, then (after 8*32 keys + 32-byte blockhash) 4 instructions.
    expect(bytes[132]).toBe(8);
    expect(bytes[132 + 1 + 8 * 32 + 32]).toBe(4);
  });

  it("rejects a fee payer equal to the paying authority (fee-payer isolation)", () => {
    expect(() =>
      buildExactSvmTransferTransaction({
        ...baseOptions,
        feePayer: "GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB",
      }),
    ).toThrow(/fee payer must differ/i);
  });

  it("generates a >=16-byte hex memo nonce when none is supplied", () => {
    const { memo } = buildExactSvmTransferTransaction({
      ...baseOptions,
      memo: undefined,
    });
    expect(memo).toMatch(/^[0-9a-f]{32}$/);
  });
});
