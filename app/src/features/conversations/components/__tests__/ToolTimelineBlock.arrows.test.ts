import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// Resolved from the project root: `import.meta.url` is not a file URL under
// vitest's transform, so `new URL(..., import.meta.url)` cannot be read.
const source = readFileSync(
  resolve(process.cwd(), 'src/features/conversations/components/ToolTimelineBlock.tsx'),
  'utf8'
);

/**
 * The disclosure arrows rotate via a Tailwind group variant, and the variant has
 * to name the same group the collapsible root declares.
 *
 * They were all written as the unnamed `group-data-[state=open]:` while every
 * root in this file declares a *named* group (`group/resp`, `group/row`,
 * `group/insights`). Tailwind's unnamed variant only matches an ancestor with a
 * bare `group` class, so none of them ever rotated — the arrow sat still while
 * the section expanded. Nothing failed; the style simply never applied, which is
 * why this is asserted on the source rather than on a rendered DOM.
 */
describe('ToolTimelineBlock disclosure arrows', () => {
  it('scopes every rotate variant to a group this file declares', () => {
    const declared = new Set([...source.matchAll(/\bgroup\/([A-Za-z0-9_-]+)/g)].map(m => m[1]));
    expect(declared.size).toBeGreaterThan(0);

    const variants = [...source.matchAll(/group-data-\[state=open\](\/([A-Za-z0-9_-]+))?:/g)];
    expect(variants.length).toBeGreaterThan(0);

    for (const match of variants) {
      const name = match[2];
      expect(name, `unnamed group variant in "${match[0]}" matches no named root`).toBeDefined();
      expect(declared.has(name!), `"${name}" is not a group declared in this file`).toBe(true);
    }
  });
});
