import type { resources, defaultNS } from "@src/common/i18n";

declare module "i18next" {
	interface CustomTypeOptions {
		defaultNS: typeof defaultNS;
		resources: (typeof resources)["en"];
	}
}
