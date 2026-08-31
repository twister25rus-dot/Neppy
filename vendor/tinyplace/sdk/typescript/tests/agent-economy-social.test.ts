import { describe, expect, it } from "vitest";
import {
  resolveOwnHandle,
  subscribeBroadcast,
} from "../src/agent/index.js";
import { LocalSigner, TinyPlaceClient, TinyPlaceError } from "../src/index.js";

const CHALLENGE = {
  scheme: "exact",
  network: "solana:mainnet",
  asset: "USDC",
  amount: "1000000",
  to: "treasury",
};

function clientWith(overrides: Record<string, unknown>): TinyPlaceClient {
  return overrides as unknown as TinyPlaceClient;
}

async function signer(): Promise<LocalSigner> {
  return LocalSigner.generate({ siws: false });
}

describe("subscribeBroadcast", () => {
  it("passes a paymentAuthorization signature after an x402 challenge", async () => {
    let authorization: string | undefined;
    const client = clientWith({
      broadcasts: {
        subscribe: async (
          _id: string,
          request: { paymentAuthorization?: string },
        ) => {
          if (!request.paymentAuthorization) {
            throw new TinyPlaceError(402, { payment: CHALLENGE });
          }
          authorization = request.paymentAuthorization;
          return {
            agentId: "me",
            status: "active",
            subscribedAt: "2026-06-20T00:00:00.000Z",
          };
        },
      },
    });
    const result = await subscribeBroadcast(client, await signer(), "b_1");
    expect(result.status).toBe("active");
    expect(authorization).toBeTruthy();
  });
});

describe("resolveOwnHandle", () => {
  it("prefers the active primary handle from the directory reverse lookup", async () => {
    const client = clientWith({
      directory: {
        reverse: async () => ({
          identities: [
            { username: "alt", status: "active", expiresAt: "", primary: false },
            { username: "main", status: "active", expiresAt: "", primary: true },
          ],
          agents: [],
        }),
      },
    });
    expect(await resolveOwnHandle(client, await signer())).toBe("@main");
  });

  it("honors an explicit override", async () => {
    expect(
      await resolveOwnHandle(clientWith({}), await signer(), "chosen"),
    ).toBe("@chosen");
  });
});
