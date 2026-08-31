import type { Metadata } from "next";

import { SectionPage } from "@src/components/layout/SectionPage";

export const metadata: Metadata = {
	title: "On-ramp / Off-ramp",
	description:
		"Fund your SOL wallet with USDC on tiny.place, powered by MoonPay.",
};

// The open tab lives in the URL (e.g. /onramp/offramp); SectionPage's component
// reads it from the path, so this route renders the same view.
export default function Page(): React.ReactElement {
	return <SectionPage section="onramp" />;
}
