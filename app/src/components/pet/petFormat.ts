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

/** What the indicator shows: the four core states, with "observing screen" split out. */
export type CompanionDisplayState =
  | 'off'
  | 'observing'
  | 'observingScreen'
  | 'paused'
  | 'suspended';

export function companionDisplayState(
  state: 'off' | 'observing' | 'paused' | 'suspended' | null | undefined,
  screenCaptureActive: boolean | null | undefined
): CompanionDisplayState {
  switch (state) {
    case 'observing':
      return screenCaptureActive ? 'observingScreen' : 'observing';
    case 'paused':
    case 'suspended':
      return state;
    default:
      return 'off';
  }
}

/**
 * Human form of a Tauri accelerator, for display only. `CmdOrCtrl+Alt+Shift+P`
 * becomes `⌥⇧⌘P` on macOS and `Ctrl+Alt+Shift+P` elsewhere. An empty or missing
 * accelerator returns `null` (the shortcut is disabled).
 */
export function formatHotkey(accel: string | null | undefined, isMac: boolean): string | null {
  if (!accel || !accel.trim()) return null;
  const parts = accel
    .split('+')
    .map(p => p.trim())
    .filter(Boolean);
  const key = parts.pop() ?? '';
  const mods = new Set(parts.map(p => p.toLowerCase()));
  const has = (...names: string[]) => names.some(n => mods.has(n));
  if (isMac) {
    const glyphs = [
      has('ctrl', 'control') ? '⌃' : '',
      has('alt', 'option') ? '⌥' : '',
      has('shift') ? '⇧' : '',
      has('cmd', 'command', 'super', 'meta', 'cmdorctrl', 'commandorcontrol') ? '⌘' : '',
    ].join('');
    return `${glyphs}${key.length === 1 ? key.toUpperCase() : key}`;
  }
  const names = [
    has('ctrl', 'control', 'cmdorctrl', 'commandorcontrol') ? 'Ctrl' : '',
    has('alt', 'option') ? 'Alt' : '',
    has('shift') ? 'Shift' : '',
    has('cmd', 'command', 'super', 'meta') ? 'Win' : '',
  ].filter(Boolean);
  return [...names, key.length === 1 ? key.toUpperCase() : key].join('+');
}

export const MAX_TITLE_RULES = 100;
export const MAX_TITLE_RULE_LEN = 200;

/** Client-side mirror of the core's title-rule validation. `null` means valid. */
export function titleRuleProblem(
  rule: string,
  existing: readonly string[]
): 'tooLong' | 'tooMany' | 'badRegex' | null {
  if (rule.length > MAX_TITLE_RULE_LEN) return 'tooLong';
  if (existing.length >= MAX_TITLE_RULES) return 'tooMany';
  if (rule.startsWith('re:')) {
    try {
      new RegExp(rule.slice(3));
    } catch {
      return 'badRegex';
    }
  }
  return null;
}
