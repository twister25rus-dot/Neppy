import { describe, expect, it } from 'vitest';

import { parseUnifiedDiff } from './parseUnifiedDiff';

const MULTI = [
  'diff --git a/src/a.ts b/src/a.ts',
  'index 111..222 100644',
  '--- a/src/a.ts',
  '+++ b/src/a.ts',
  '@@ -1,3 +1,3 @@',
  ' keep',
  '-old',
  '+new',
  ' tail',
  'diff --git a/src/b.ts b/src/b.ts',
  'index 333..444 100644',
  '--- a/src/b.ts',
  '+++ b/src/b.ts',
  '@@ -5 +5,2 @@',
  ' ctx',
  '+extra',
  '',
].join('\n');

describe('parseUnifiedDiff', () => {
  it('returns nothing for empty text', () => {
    expect(parseUnifiedDiff('')).toEqual([]);
  });

  it('splits multiple files and counts +/- lines', () => {
    const files = parseUnifiedDiff(MULTI);
    expect(files.map(f => f.path)).toEqual(['src/a.ts', 'src/b.ts']);
    expect(files[0]).toMatchObject({ added: 1, removed: 1, status: 'modified', binary: false });
    expect(files[1]).toMatchObject({ added: 1, removed: 0 });
    expect(files[0].lines.map(l => l.kind)).toEqual(['hunk', 'context', 'del', 'add', 'context']);
    expect(files[0].lines[2].text).toBe('old');
  });

  it('detects a new file', () => {
    const files = parseUnifiedDiff(
      [
        'diff --git a/n.txt b/n.txt',
        'new file mode 100644',
        'index 0000000..abc',
        '--- /dev/null',
        '+++ b/n.txt',
        '@@ -0,0 +1,2 @@',
        '+one',
        '+two',
        '',
      ].join('\n')
    );
    expect(files).toHaveLength(1);
    expect(files[0]).toMatchObject({ path: 'n.txt', status: 'created', added: 2, removed: 0 });
  });

  it('detects a deleted file and keeps its path', () => {
    const files = parseUnifiedDiff(
      [
        'diff --git a/gone.txt b/gone.txt',
        'deleted file mode 100644',
        'index abc..0000000',
        '--- a/gone.txt',
        '+++ /dev/null',
        '@@ -1 +0,0 @@',
        '-bye',
        '',
      ].join('\n')
    );
    expect(files[0]).toMatchObject({ path: 'gone.txt', status: 'deleted', added: 0, removed: 1 });
  });

  it('marks binary files without lines', () => {
    const files = parseUnifiedDiff(
      [
        'diff --git a/img.png b/img.png',
        'index 1..2 100644',
        'Binary files a/img.png and b/img.png differ',
        '',
      ].join('\n')
    );
    expect(files[0]).toMatchObject({ path: 'img.png', binary: true, added: 0, removed: 0 });
    expect(files[0].lines).toEqual([]);
  });

  it('keeps the no-newline marker as a note, not a change', () => {
    const files = parseUnifiedDiff(
      [
        'diff --git a/x b/x',
        '--- a/x',
        '+++ b/x',
        '@@ -1 +1 @@',
        '-a',
        '\\ No newline at end of file',
        '+b',
        '\\ No newline at end of file',
        '',
      ].join('\n')
    );
    expect(files[0].added).toBe(1);
    expect(files[0].removed).toBe(1);
    expect(files[0].lines.filter(l => l.kind === 'note')).toHaveLength(2);
  });

  it('treats +++/--- lines inside a hunk as content', () => {
    const files = parseUnifiedDiff(
      ['diff --git a/m b/m', '--- a/m', '+++ b/m', '@@ -1 +1 @@', '--- x', '+++ y', ''].join('\n')
    );
    expect(files[0]).toMatchObject({ added: 1, removed: 1 });
  });

  it('records the old path on a rename', () => {
    const files = parseUnifiedDiff(
      [
        'diff --git a/old.ts b/new.ts',
        'similarity index 100%',
        'rename from old.ts',
        'rename to new.ts',
        '',
      ].join('\n')
    );
    expect(files[0]).toMatchObject({ path: 'new.ts', oldPath: 'old.ts', status: 'renamed' });
  });
});
