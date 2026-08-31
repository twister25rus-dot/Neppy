import type { Signer } from "@tinyhumansai/tinyplace";

import {
	signX402ChallengePaymentMap,
	type X402ChallengePayment,
} from "@src/common/auth-payment";
import type { ExpectedX402Payment } from "@src/common/x402-challenge";
import type {
	X402ConfirmContextValue,
	X402ConfirmRequest,
} from "@src/components/explore/x402-confirm";

type ConfirmX402 = X402ConfirmContextValue["confirm"] | undefined;

export type ConfirmAndSettleX402Options<T> = {
	/** The 402 challenge payment to authorize. */
	payment: X402ChallengePayment;
	/** The signer (normally the hot session signer). */
	signer: Signer;
	/** Payer cryptoId used when the challenge omits `from`. */
	fallbackFrom: string;
	/** Prefix for the generated payment nonce, e.g. "reg", "buy". */
	noncePrefix: string;
	/** Extra x402 metadata merged into the signed authorization. */
	metadata?: Record<string, string>;
	/** Optional client-side guard on the challenge's money-bearing fields. */
	expected?: ExpectedX402Payment;
	/**
	 * Submits the signed payment and resolves once it has SETTLED — i.e. the
	 * backend has confirmed the on-chain transaction (it waits for finalization).
	 * Its resolved value is returned by {@link confirmAndSettleX402}.
	 */
	submit: (payment: Record<string, string>) => Promise<T>;
	/** When present (with `confirmRequest`), routes the flow through the dialog. */
	confirmX402?: ConfirmX402;
	/** The dialog descriptor (title/subject/note/label). */
	confirmRequest?: X402ConfirmRequest;
};

/**
 * The standard x402 flow used across the app: sign the challenge, then submit it
 * — running BOTH inside the confirm dialog's action so the dialog stays in
 * "Confirming…" until the payment is actually settled on-chain (not merely
 * signed), shows "Payment confirmed" only after settlement, and surfaces a
 * settlement failure as a retryable dialog error. Without a dialog it just signs
 * and submits. Returns whatever `submit` resolves to (the settled result).
 */
export async function confirmAndSettleX402<T>(
	options: ConfirmAndSettleX402Options<T>
): Promise<T> {
	const settle = async (): Promise<T> => {
		const payment = await signX402ChallengePaymentMap({
			expected: options.expected,
			fallbackFrom: options.fallbackFrom,
			metadata: options.metadata,
			noncePrefix: options.noncePrefix,
			payment: options.payment,
			signer: options.signer,
		});
		return options.submit(payment);
	};

	if (!options.confirmX402 || !options.confirmRequest) {
		return settle();
	}
	return (await options.confirmX402(
		{
			amount: options.payment.amount,
			asset: options.payment.asset,
			recipient: options.payment.to,
			...options.confirmRequest,
		},
		settle
	)) as T;
}
