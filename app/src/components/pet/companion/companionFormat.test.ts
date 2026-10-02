import { describe, expect, it } from 'vitest';

import { companionDisplayState, formatHotkey, titleRuleProblem } from '../petFormat';

describe('companionDisplayState', () => {
  it('splits observing into observing and observing screen', () => {
    expect(companionDisplayState('observing', false)).toBe('observing');
    expect(companionDisplayState('observing', true)).toBe('observingScreen');
  });
  it('ignores the screen flag outside observing', () => {
    expect(companionDisplayState('paused', true)).toBe('paused');
    expect(companionDisplayState('suspended', true)).toBe('suspended');
    expect(companionDisplayState('off', true)).toBe('off');
    expect(companionDisplayState(null, true)).toBe('off');
  });
});

describe('formatHotkey', () => {
  it('uses glyphs on macOS', () => {
    expect(formatHotkey('CmdOrCtrl+Alt+Shift+P', true)).toBe('⌥⇧⌘P');
    expect(formatHotkey('CmdOrCtrl+Alt+Shift+Space', true)).toBe('⌥⇧⌘Space');
  });
  it('uses names elsewhere', () => {
    expect(formatHotkey('CmdOrCtrl+Alt+Shift+S', false)).toBe('Ctrl+Alt+Shift+S');
  });
  it('treats an empty or missing accelerator as disabled', () => {
    expect(formatHotkey('', true)).toBeNull();
    expect(formatHotkey(null, true)).toBeNull();
    expect(formatHotkey(undefined, false)).toBeNull();
  });
});

describe('titleRuleProblem', () => {
  it('accepts plain text and valid regexes', () => {
    expect(titleRuleProblem('incognito', [])).toBeNull();
    expect(titleRuleProblem('re:^Bank', [])).toBeNull();
  });
  it('rejects bad regexes, long rules and too many rules', () => {
    expect(titleRuleProblem('re:(', [])).toBe('badRegex');
    expect(titleRuleProblem('x'.repeat(201), [])).toBe('tooLong');
    expect(
      titleRuleProblem(
        'x',
        Array.from({ length: 100 }, (_, i) => `r${i}`)
      )
    ).toBe('tooMany');
  });
});
