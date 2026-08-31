import type { StorybookConfig } from "@storybook/react-vite";
const config: StorybookConfig = {
	stories: ["../src/**/*.mdx", "../src/**/*.stories.@(js|jsx|ts|tsx)"],
	addons: [
		"@storybook/addon-links",
		"@storybook/addon-themes",
		"@storybook/addon-docs",
	],
	framework: {
		name: "@storybook/react-vite",
		options: {},
	},
	docs: {
		defaultName: "Documentation",
		docsMode: true,
	},
};
export default config;
