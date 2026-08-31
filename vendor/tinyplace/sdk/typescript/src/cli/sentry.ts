import * as Sentry from "@sentry/node";

const CLI_SENTRY_DSN_ENV = "TINYPLACE_CLI_SENTRY_DSN";

let initializedDsn: string | undefined;

export function initCliSentry(
  env: Record<string, string | undefined>,
): boolean {
  const dsn = env[CLI_SENTRY_DSN_ENV]?.trim();
  if (!dsn) {
    return false;
  }
  if (initializedDsn === dsn) {
    return true;
  }
  Sentry.init({
    dsn,
    environment: env.TINYPLACE_ENV ?? env.NODE_ENV,
    sendDefaultPii: true,
  });
  initializedDsn = dsn;
  return true;
}

export function captureCliException(error: unknown, command?: string): void {
  if (!initializedDsn) {
    return;
  }
  Sentry.captureException(error, (scope) => {
    if (command) {
      scope.setTag("tinyplace.cli.command", command);
    }
    return scope;
  });
}

export async function flushCliSentry(timeout = 2000): Promise<void> {
  if (!initializedDsn) {
    return;
  }
  await Sentry.flush(timeout);
}
