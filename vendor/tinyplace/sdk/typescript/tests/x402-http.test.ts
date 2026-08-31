import { describe, expect, it } from "vitest";

import { HttpClient } from "../src/http.js";
import { SOLANA_USDC_MINT } from "../src/solana.js";
import {
  encodeX402Header,
  type X402PaymentPayload,
  type X402SettlementResponse,
} from "../src/x402-standard.js";

const FEE_PAYER = "EwWqGE4ZFKLofuestmU4LDdK7XM1N4ALgdZccwYugwGd";
const PAY_TO = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const NETWORK = "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp";

function challengeHeader(): string {
  return encodeX402Header({
    x402Version: 2,
    error: "payment required",
    resource: { url: "/registry/names" },
    accepts: [
      {
        scheme: "exact",
        network: NETWORK,
        amount: "1000000",
        asset: SOLANA_USDC_MINT,
        payTo: PAY_TO,
        maxTimeoutSeconds: 60,
        extra: { feePayer: FEE_PAYER },
      },
    ],
  });
}

/**
 * A scripted fetch: blockhash RPC always succeeds; the first call to the gated
 * resource 402s with a PAYMENT-REQUIRED header, and the retry (carrying a
 * PAYMENT-SIGNATURE) 200s with a PAYMENT-RESPONSE settlement.
 */
function scriptedFetch(): {
  fetch: typeof globalThis.fetch;
  calls: () => Array<{ url: string; paymentSignature?: string }>;
} {
  const calls: Array<{ url: string; paymentSignature?: string }> = [];
  const fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const headers = new Headers(init?.headers);
    if (url.includes("rpc.test")) {
      return new Response(
        JSON.stringify({
          jsonrpc: "2.0",
          id: "getLatestBlockhash",
          result: { value: { blockhash: "11111111111111111111111111111111" } },
        }),
        { status: 200 },
      );
    }
    const paymentSignature = headers.get("PAYMENT-SIGNATURE") ?? undefined;
    calls.push({ url, ...(paymentSignature ? { paymentSignature } : {}) });
    if (!paymentSignature) {
      return new Response("{}", {
        status: 402,
        headers: { "PAYMENT-REQUIRED": challengeHeader() },
      });
    }
    const settlement: X402SettlementResponse = {
      success: true,
      transaction: "5xSettledSig",
      network: NETWORK,
      payer: PAY_TO,
    };
    return new Response(JSON.stringify({ username: "alice", registered: true }), {
      status: 200,
      headers: {
        "Content-Type": "application/json",
        "PAYMENT-RESPONSE": encodeX402Header(settlement),
      },
    });
  }) as unknown as typeof globalThis.fetch;
  return { fetch, calls: () => calls };
}

describe("standard x402 auto-payment over HTTP", () => {
  it("retries a 402 with a PAYMENT-SIGNATURE and returns the resource", async () => {
    const scripted = scriptedFetch();
    let settled: X402SettlementResponse | undefined;

    const client = new HttpClient({
      baseUrl: "https://api.test.invalid",
      fetch: scripted.fetch,
      x402Payer: {
        secretKey: new Uint8Array(32).fill(7),
        rpcUrl: "https://rpc.test.invalid",
        fetch: scripted.fetch,
        onSettled: (s) => {
          settled = s;
        },
      },
    });

    const result = await client.post<{ username: string; registered: boolean }>(
      "/registry/names",
      { username: "alice" },
    );
    expect(result).toMatchObject({ username: "alice", registered: true });

    const calls = scripted.calls();
    expect(calls).toHaveLength(2);
    expect(calls[0]?.paymentSignature).toBeUndefined();
    expect(typeof calls[1]?.paymentSignature).toBe("string");

    // The PAYMENT-SIGNATURE decodes to a v2 payload carrying the transfer tx.
    const payload = JSON.parse(
      new TextDecoder().decode(
        Uint8Array.from(atob(calls[1]!.paymentSignature!), (c) =>
          c.charCodeAt(0),
        ),
      ),
    ) as X402PaymentPayload;
    expect(payload.x402Version).toBe(2);
    expect(payload.accepted.asset).toBe(SOLANA_USDC_MINT);
    expect(typeof payload.payload["transaction"]).toBe("string");

    // The settlement (PAYMENT-RESPONSE) is surfaced to the payer hook.
    expect(settled?.success).toBe(true);
    expect(settled?.transaction).toBe("5xSettledSig");
  });

  it("surfaces a 402 as an error when no payer is configured", async () => {
    const scripted = scriptedFetch();
    const client = new HttpClient({
      baseUrl: "https://api.test.invalid",
      fetch: scripted.fetch,
    });
    await expect(
      client.post("/registry/names", { username: "bob" }),
    ).rejects.toThrow(/HTTP 402/);
  });
});
