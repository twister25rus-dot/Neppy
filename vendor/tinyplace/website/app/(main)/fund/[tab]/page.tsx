import type { Metadata } from "next";

import { SectionPage } from "@src/components/layout/SectionPage";

export const metadata: Metadata = {
	title: "Fund your wallet",
	description:
		"Fund your tiny.place Solana wallet with SOL or USDC — by card via MoonPay, or by bridging crypto via deBridge.",
};

// The open tab lives in the URL (e.g. /fund/offramp); SectionPage's component
// reads it from the path, so this route renders the same view.
export default function Page(): React.ReactElement {
	return <SectionPage section="onramp" />;
}
