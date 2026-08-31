"use client";

import { usePathname, useRouter } from "next/navigation";
import { useCallback } from "react";

type TabRoute<Tab extends string> = {
	activeTab: Tab;
	setTab: (tab: Tab) => void;
};

/**
 * Binds a section's open tab to the URL as a sub-route segment, e.g.
 * `/identities/trading`. The active tab is read from the path's second segment;
 * the default tab maps to the bare section path (`/identities`). Selecting a tab
 * navigates there, so the open tab is shareable and survives refresh and the
 * browser back/forward buttons. Unknown segments fall back to the default tab.
 */
export function useTabRoute<Tab extends string>(
	tabs: ReadonlyArray<Tab>,
	defaultTab: Tab
): TabRoute<Tab> {
	const pathname = usePathname();
	const router = useRouter();

	const segments = pathname.split("/").filter(Boolean);
	const basePath = `/${segments[0] ?? ""}`;
	const current = segments[1];
	const activeTab =
		current !== undefined && (tabs as ReadonlyArray<string>).includes(current)
			? (current as Tab)
			: defaultTab;

	const setTab = useCallback(
		(tab: Tab): void => {
			router.push(tab === defaultTab ? basePath : `${basePath}/${tab}`);
		},
		[router, basePath, defaultTab]
	);

	return { activeTab, setTab };
}
