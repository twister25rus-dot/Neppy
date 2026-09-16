import { describe, expect, it } from 'vitest';

interface LocaleModule {
  default: Record<string, string>;
}

const LOCALES = [
  'en',
  'zh-CN',
  'hi',
  'es',
  'ar',
  'fr',
  'bn',
  'pt',
  'de',
  'ru',
  'id',
  'it',
  'ko',
  'pl',
] as const;

/** Same eager glob `coverage.test.ts` uses — no runtime dynamic import. */
const localeModules = import.meta.glob<LocaleModule>('../*.ts', { eager: true });

function loadLocale(locale: string): Record<string, string> {
  const mod = localeModules[`../${locale}.ts`];
  if (!mod) throw new Error(`missing locale file: ${locale}.ts`);
  return mod.default;
}

/**
 * Keys the app interpolates with `.replace('{name}', …)`. A value that has lost
 * its placeholder fails nothing — not the build, not `tsc`, not `i18n:check`,
 * which only compares key SETS. It just renders a sentence with the number
 * silently missing.
 *
 * Both of the first entry's failures were live. English carried
 * `'${formatBytes(downloaded)} downloaded'` — a template literal that was never
 * a template — so an English user whose updater reported no total read that
 * source text verbatim in the progress row. zh-CN had dropped `{amount}`
 * altogether and rendered "downloaded" with no size at all.
 */
const REQUIRED_PLACEHOLDERS: Record<string, string[]> = {
  'app.update.progress.downloaded': ['{amount}'],
  'app.update.progress.working': ['{percent}'],
  'app.update.versionAvailable': ['{newVersion}'],
  'app.update.currentlyOn': ['{version}'],
  'mlx.memoryUsage': ['{used}', '{budget}'],
};

describe('i18n interpolation placeholders', () => {
  it.each(LOCALES)('locale %s keeps every interpolation placeholder', locale => {
    const flat = loadLocale(locale);
    const broken: string[] = [];
    for (const [key, placeholders] of Object.entries(REQUIRED_PLACEHOLDERS)) {
      const value = flat[key];
      // An untranslated key is a different concern, already covered by
      // `coverage.test.ts`.
      if (value === undefined) continue;
      for (const placeholder of placeholders) {
        if (!value.includes(placeholder)) broken.push(`${key} is missing ${placeholder}`);
      }
    }
    expect(broken).toEqual([]);
  });

  it.each(LOCALES)('locale %s leaks no source code into copy', locale => {
    // `${amount}` is legitimate and common here: a literal dollar sign before a
    // placeholder, as in "$12.34 this month". A CALL is never copy, and that is
    // what separates the real defect from those.
    const flat = loadLocale(locale);
    const leaks = Object.entries(flat)
      .filter(([, value]) => typeof value === 'string' && /\$\{[^}]*\(/.test(value))
      .map(([key, value]) => `${key} -> ${value}`);
    expect(leaks).toEqual([]);
  });
});
