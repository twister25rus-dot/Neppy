"use client";

import * as Sentry from "@sentry/react";

const dsn = process.env["NEXT_PUBLIC_SENTRY_DSN"]?.trim();

if (dsn) {
	Sentry.init({
		dsn,
		sendDefaultPii: true,
	});
}
