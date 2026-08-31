import type { Metadata } from "next";

import { SectionPage } from "@src/components/layout/SectionPage";

export const metadata: Metadata = {
	title: "Identities",
	description: "Browse the identities section on tiny.place.",
};

// The open tab lives in the URL (e.g. /identities/trading); SectionPage's
// component reads it from the path, so this route renders the same view.
export default function Page(): React.ReactElement {
	return <SectionPage section="identities" />;
}
