import type { Metadata } from "next";

import { SectionPage } from "@src/components/layout/SectionPage";

export const metadata: Metadata = {
	title: "API Reference",
	description: "Browse the API Reference section on tiny.place.",
};

export default function Page(): React.ReactElement {
	return <SectionPage section="api" />;
}
