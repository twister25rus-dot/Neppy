import type { BadgeVariant } from '../ui/Badge';

/** Strict 24-hour `HH:MM`, the only time format the core accepts. */
const HHMM = /^([01]\d|2[0-3]):[0-5]\d$/;

export const isValidHHMM = (value: string): boolean => HHMM.test(value);

const UNITS: Array<[Intl.RelativeTimeFormatUnit, number]> = [
  ['year', 365 * 24 * 3600],
  ['month', 30 * 24 * 3600],
  ['day', 24 * 3600],
  ['hour', 3600],
  ['minute', 60],
];

/**
 * Locale-aware relative time ("in 2 days", "5 minutes ago") for an RFC3339
 * string. Returns `null` when the input is missing or unparseable so the caller
 * can substitute its own translated placeholder.
 */
export function formatRelative(
  iso: string | null | undefined,
  locale: string,
  now: number = Date.now()
): string | null {
  if (!iso) return null;
  const ts = Date.parse(iso);
  if (Number.isNaN(ts)) return null;
  const diffSec = Math.round((ts - now) / 1000);
  const abs = Math.abs(diffSec);
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  for (const [unit, size] of UNITS) {
    if (abs >= size) return rtf.format(Math.round(diffSec / size), unit);
  }
  return rtf.format(Math.round(diffSec / 60), 'minute');
}

/** Locale-aware short date and time for an RFC3339 string, or `null`. */
export function formatDateTime(iso: string | null | undefined, locale: string): string | null {
  if (!iso) return null;
  const ts = Date.parse(iso);
  if (Number.isNaN(ts)) return null;
  return new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(
    new Date(ts)
  );
}

export function noteStateVariant(state: string): BadgeVariant {
  switch (state) {
    case 'notified':
      return 'warning';
    case 'queued':
      return 'primary';
    case 'digested':
      return 'success';
    default:
      return 'neutral';
  }
}

export function urgencyVariant(urgency: number): BadgeVariant {
  if (urgency >= 3) return 'danger';
  if (urgency === 2) return 'warning';
  return 'neutral';
}

/** Best-effort message text from an unknown thrown value (core error text only). */
export function errorText(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  return '';
}
