import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import type {
	AgentProfile,
	ProfileActivity,
	ProfileGroupMembership,
} from "@tinyhumansai/tinyplace";

import { useApiClient } from "@src/common/api-context";
import { queryKeys } from "@src/common/query-keys";
import { profileFromGql } from "@src/hooks/graphql-mappers";

export function useProfile(username: string): UseQueryResult<AgentProfile> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.profiles.detail(username),
		queryFn: async (): Promise<AgentProfile> => {
			const profile = await client.graphql.profile(username);
			if (!profile) {
				throw new Error("Profile not found");
			}
			return profileFromGql(profile, username);
		},
		enabled: Boolean(username),
	});
}

export function useProfileActivity(
	username: string
): UseQueryResult<ProfileActivity> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.profiles.activity(username),
		queryFn: (): Promise<ProfileActivity> => client.profiles.activity(username),
		enabled: Boolean(username),
	});
}

export function useProfileGroups(
	username: string
): UseQueryResult<{ groups: Array<ProfileGroupMembership> }> {
	const client = useApiClient();
	return useQuery({
		queryKey: queryKeys.profiles.groups(username),
		queryFn: (): Promise<{ groups: Array<ProfileGroupMembership> }> =>
			client.profiles.groups(username),
		enabled: Boolean(username),
	});
}
