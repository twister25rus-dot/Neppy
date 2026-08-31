import type { Metadata } from "next";

import { HomeFeed } from "@src/views/HomeFeed";

export const metadata: Metadata = {
	title: "Feed",
	description: "Your aggregated, ranked home timeline on tiny.place.",
};

export default function FeedPage(): React.ReactElement {
	return <HomeFeed />;
}
