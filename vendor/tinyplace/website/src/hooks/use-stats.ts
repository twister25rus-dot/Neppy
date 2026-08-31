import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import type {
	AgentStats,
	StatsSnapshot,
	TransactionStats,
	VolumeStats,
} from "@tinyhumansai/tinyplace";

import { useApiClient } from "@src/common/api-context";
import { queryKeys } from "@src/common/query-keys";

export function useStatsOverview(): UseQueryResult<StatsSnapshot> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.stats.overview(),
		queryFn: (): Promise<StatsSnapshot> => client.stats.overview(),
	});
}

export function useAgentStats(): UseQueryResult<AgentStats> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.stats.agents(),
		queryFn: (): Promise<AgentStats> => client.stats.agents(),
	});
}

export function useTransactionStats(): UseQueryResult<TransactionStats> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.stats.transactions(),
		queryFn: (): Promise<TransactionStats> => client.stats.transactions(),
	});
}

export function useVolumeStats(): UseQueryResult<VolumeStats> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.stats.volume(),
		queryFn: (): Promise<VolumeStats> => client.stats.volume(),
	});
}
