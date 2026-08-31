import { describe, expect, it } from "vitest";
import { TinyPlaceClient, formatTokenAmount } from "../src/index.js";

describe("SolanaApi", () => {
  it("fetches public chain metadata", async () => {
    const requests: Array<Request> = [];
    const client = new TinyPlaceClient({
      baseUrl: "https://example.test",
      fetch: async (input, init) => {
        requests.push(new Request(input, init));
        return Response.json({
          network: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
          name: "Solana",
          kind: "solana",
          nativeAsset: "SOL",
          explorerUrl: "https://solscan.io",
          confirmations: 32,
          assets: [{ symbol: "SOL", decimals: 9 }],
          rpc: {
            url: "https://example.test/solana/rpc",
            rateLimitPerMin: 20,
            fallbacks: true,
          },
        });
      },
    });

    const info = await client.solana.info();

    expect(info.rpc.url).toBe("https://example.test/solana/rpc");
    expect(requests).toHaveLength(1);
    expect(requests[0]!.method).toBe("GET");
    expect(requests[0]!.url).toBe("https://example.test/solana");
  });

  it("posts JSON-RPC requests to the tiny.place proxy", async () => {
    const requests: Array<Request> = [];
    const client = new TinyPlaceClient({
      baseUrl: "https://example.test",
      fetch: async (input, init) => {
        const request = new Request(input, init);
        requests.push(request);
        return Response.json({
          jsonrpc: "2.0",
          id: "getHealth",
          result: "ok",
        });
      },
    });

    const response = await client.solana.rpc<string>({
      jsonrpc: "2.0",
      id: "getHealth",
      method: "getHealth",
    });

    expect(response.result).toBe("ok");
    expect(requests).toHaveLength(1);
    expect(requests[0]!.method).toBe("POST");
    expect(requests[0]!.url).toBe("https://example.test/solana/rpc");
    await expect(requests[0]!.json()).resolves.toEqual({
      jsonrpc: "2.0",
      id: "getHealth",
      method: "getHealth",
    });
  });

  it("unwraps JSON-RPC results with call", async () => {
    const client = new TinyPlaceClient({
      baseUrl: "https://example.test",
      fetch: async () =>
        Response.json({
          jsonrpc: "2.0",
          id: "getSlot",
          result: 123,
        }),
    });

    await expect(client.solana.call<number>("getSlot")).resolves.toBe(123);
  });

  it("throws JSON-RPC errors from call", async () => {
    const client = new TinyPlaceClient({
      baseUrl: "https://example.test",
      fetch: async () =>
        Response.json({
          jsonrpc: "2.0",
          id: "getSlot",
          error: { code: -32005, message: "rate limit" },
        }),
    });

    await expect(client.solana.call("getSlot")).rejects.toThrow(
      "Solana JSON-RPC -32005: rate limit",
    );
  });

  it("reads native + SPL balances with single (non-batch) calls", async () => {
    const methods: Array<string> = [];
    const client = new TinyPlaceClient({
      baseUrl: "https://example.test",
      fetch: async (input, init) => {
        const request = new Request(input, init);
        if (request.url.endsWith("/solana")) {
          return Response.json({
            network: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
            name: "Solana",
            kind: "solana",
            nativeAsset: "SOL",
            explorerUrl: "https://solscan.io",
            confirmations: 32,
            assets: [
              { symbol: "SOL", decimals: 9 },
              {
                symbol: "USDC",
                address: "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
                decimals: 6,
              },
            ],
            rpc: {
              url: "https://example.test/solana/rpc",
              rateLimitPerMin: 20,
              fallbacks: true,
            },
          });
        }
        const body = (await request.json()) as { method: string; id: string };
        // The proxy's free plan rejects arrays; assert each call is a lone object.
        expect(Array.isArray(body)).toBe(false);
        methods.push(body.method);
        if (body.method === "getBalance") {
          return Response.json({
            jsonrpc: "2.0",
            id: body.id,
            result: { context: { slot: 1 }, value: 1_500_000_000 },
          });
        }
        return Response.json({
          jsonrpc: "2.0",
          id: body.id,
          result: {
            context: { slot: 1 },
            value: [
              {
                account: {
                  data: {
                    parsed: { info: { tokenAmount: { amount: "2500000", decimals: 6 } } },
                  },
                },
              },
            ],
          },
        });
      },
    });

    const result = await client.solana.balances("OwnerAddr111");

    expect(methods).toEqual(["getBalance", "getTokenAccountsByOwner"]);
    expect(result.address).toBe("OwnerAddr111");
    expect(result.balances).toEqual([
      { symbol: "SOL", raw: "1500000000", decimals: 9, amount: "1.5" },
      {
        symbol: "USDC",
        mint: "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
        raw: "2500000",
        decimals: 6,
        amount: "2.5",
      },
    ]);
  });
});

describe("formatTokenAmount", () => {
  it("scales base units into decimal strings and trims zeros", () => {
    expect(formatTokenAmount(0n, 9)).toBe("0");
    expect(formatTokenAmount(1_500_000_000n, 9)).toBe("1.5");
    expect(formatTokenAmount(261_542_411_670n, 9)).toBe("261.54241167");
    expect(formatTokenAmount(1n, 6)).toBe("0.000001");
    expect(formatTokenAmount(42n, 0)).toBe("42");
    expect(formatTokenAmount(-2_500_000n, 6)).toBe("-2.5");
  });
});
