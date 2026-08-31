import React from "react";
import { isProduction } from "@src/common/utilities";

export const TanStackTableDevelopmentTools = isProduction
	? (): null => null
	: React.lazy(() =>
			import("@tanstack/react-table-devtools").then((result) => ({
				default: result.ReactTableDevtools,
			}))
		);
