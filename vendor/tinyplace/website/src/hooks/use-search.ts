import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import type {
	DiscoverResponse,
	DiscoveryCategory,
	SearchResponse,
	SuggestResponse,
} from "@tinyhumansai/tinyplace";

import { useApiClient } from "@src/common/api-context";
import { queryKeys } from "@src/common/query-keys";

export function useSearch(query: string): UseQueryResult<SearchResponse> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.search.unified(query),
		queryFn: (): Promise<SearchResponse> => client.search.unified(query),
		enabled: query.length > 0,
	});
}

export function useSearchSuggestions(
	query: string
): UseQueryResult<SuggestResponse> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.search.suggestions(query),
		queryFn: (): Promise<SuggestResponse> => client.search.suggest(query),
		enabled: query.length > 0,
	});
}

export function useTrending(limit?: number): UseQueryResult<DiscoverResponse> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.search.trending(),
		queryFn: (): Promise<DiscoverResponse> => client.search.trending(limit),
	});
}

export function useNewest(limit?: number): UseQueryResult<DiscoverResponse> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.search.newest(),
		queryFn: (): Promise<DiscoverResponse> => client.search.newest(limit),
	});
}

export function useDiscoveryCategories(): UseQueryResult<{
	categories: Array<DiscoveryCategory>;
}> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.search.categories(),
		queryFn: (): Promise<{ categories: Array<DiscoveryCategory> }> =>
			client.search.categories(),
	});
}
