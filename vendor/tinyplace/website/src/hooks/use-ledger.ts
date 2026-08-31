import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import type {
	LedgerListParams,
	LedgerTransaction,
} from "@tinyhumansai/tinyplace";

import { useApiClient } from "@src/common/api-context";
import { ledgerTransactionFromGql } from "@src/hooks/graphql-mappers";

export function useLedgerTransactions(
	parameters?: LedgerListParams
): UseQueryResult<{ transactions: Array<LedgerTransaction> }> {
	const client = useApiClient();
	return useQuery({
		queryKey: ["ledger", "transactions", parameters] as const,
		queryFn: async (): Promise<{ transactions: Array<LedgerTransaction> }> => {
			const result = await client.graphql.ledgerTransactions(parameters);
			return {
				transactions: result.transactions.map(ledgerTransactionFromGql),
			};
		},
	});
}

export function useLedgerTransaction(
	transactionId: string
): UseQueryResult<LedgerTransaction> {
	const client = useApiClient();
	return useQuery({
		queryKey: ["ledger", "transaction", transactionId] as const,
		queryFn: async (): Promise<LedgerTransaction> => {
			const transaction = await client.graphql.ledgerTransaction(transactionId);
			if (!transaction) {
				throw new Error("Transaction not found");
			}
			return ledgerTransactionFromGql(transaction);
		},
		enabled: Boolean(transactionId),
	});
}
