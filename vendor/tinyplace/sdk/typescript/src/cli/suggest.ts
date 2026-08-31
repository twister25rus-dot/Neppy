import type { PaymentRequiredChallenge } from "../http.js";
import { boolFlag } from "./args.js";
import type { Flags, JsonObject } from "./types.js";

/**
 * A ready-to-run next step. Workflow returns carry an array of these so an agent
 * can act without composing the command itself — `run` is a full, pre-filled
 * `tinyplace …` invocation (ids and flags already substituted).
 */
export interface Suggestion {
  /** Plain-language description of what the command does. */
  do: string;
  /** The exact shell command to run. */
  run: string;
}

export function suggest(description: string, command: string): Suggestion {
  return { do: description, run: command };
}

/**
 * Pulls the x402 challenge off a thrown error, if any. Errors raised by the SDK
 * carry `paymentRequired` (and `status === 402`); everything else returns
 * undefined so the caller re-throws.
 */
export function paymentChallenge(
  error: unknown,
): PaymentRequiredChallenge | undefined {
  const detail = error as {
    status?: number;
    paymentRequired?: PaymentRequiredChallenge;
  };
  if (detail && typeof detail === "object" && detail.paymentRequired) {
    return detail.paymentRequired;
  }
  return undefined;
}

/**
 * Turns an x402 challenge into a structured, non-fatal result: the agent is told
 * what it owes and given pre-filled commands to fund and retry, rather than a
 * bare error. `command` is the action's own invocation (without `--execute`).
 */
export function paymentRequiredResult(
  action: string,
  command: string,
  challenge: PaymentRequiredChallenge,
): JsonObject {
  const payment = challenge.payment ?? {};
  const fundFlags = [
    payment.asset ? `--asset ${payment.asset}` : "",
    payment.amount ? `--amount ${payment.amount}` : "",
  ]
    .filter(Boolean)
    .join(" ");
  return {
    status: "payment-required",
    action,
    payment: {
      ...(payment.asset ? { asset: payment.asset } : {}),
      ...(payment.amount ? { amount: payment.amount } : {}),
      ...(payment.network ? { network: payment.network } : {}),
      ...(payment.to ? { to: payment.to } : {}),
    },
    suggestions: [
      suggest(
        "Fund your wallet if your balance is short",
        `tinyplace fund${fundFlags ? ` ${fundFlags}` : ""}`,
      ),
      suggest("Once funded, retry this action", `${command} --execute`),
    ],
    note: "This endpoint answered with an x402 payment challenge — it was not performed. Fund if needed, then retry.",
  };
}

/**
 * Runs a flow, converting an x402 challenge into payment guidance instead of an
 * error. Non-payment errors propagate. On success, wraps the result with any
 * follow-up suggestions the caller wants to surface.
 */
export async function runFlow(opts: {
  action: string;
  command: string;
  run: () => Promise<unknown>;
  onSuccess?: (result: unknown) => Array<Suggestion>;
}): Promise<unknown> {
  try {
    const result = await opts.run();
    const suggestions = opts.onSuccess?.(result) ?? [];
    return {
      status: "done",
      action: opts.action,
      result,
      ...(suggestions.length ? { suggestions } : {}),
    };
  } catch (error) {
    const challenge = paymentChallenge(error);
    if (challenge) {
      return paymentRequiredResult(opts.action, opts.command, challenge);
    }
    throw error;
  }
}

/**
 * Confirmation gate for paid / irreversible actions. Without `--execute` it
 * performs NOTHING and returns a preview plus the exact command to run to go
 * ahead. With `--execute` it runs the action through {@link runFlow} (so an x402
 * challenge still comes back as guidance, not a crash).
 */
export async function runPaidAction(opts: {
  flags: Flags;
  action: string;
  command: string;
  details?: JsonObject;
  run: () => Promise<unknown>;
  onSuccess?: (result: unknown) => Array<Suggestion>;
}): Promise<unknown> {
  if (!boolFlag(opts.flags, "execute")) {
    return {
      status: "needs-confirmation",
      action: opts.action,
      ...(opts.details ? { preview: opts.details } : {}),
      note: "Nothing was performed. Review the above, then re-run with --execute to go ahead.",
      suggestions: [
        suggest(`Execute: ${opts.action}`, `${opts.command} --execute`),
      ],
    };
  }
  return runFlow({
    action: opts.action,
    command: opts.command,
    run: opts.run,
    onSuccess: opts.onSuccess,
  });
}
