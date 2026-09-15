import { describe, expect, it } from 'vitest';

import { isAbsoluteFolderPath } from './folderPath';

describe('isAbsoluteFolderPath', () => {
  it('accepts a POSIX absolute path', () => {
    expect(isAbsoluteFolderPath('/Users/alex/AI Memory Hub')).toBe(true);
  });

  it('accepts Windows drive-letter and UNC roots', () => {
    expect(isAbsoluteFolderPath('C:\\Users\\alex\\Notes')).toBe(true);
    expect(isAbsoluteFolderPath('C:/Users/alex/Notes')).toBe(true);
    expect(isAbsoluteFolderPath('\\\\server\\share')).toBe(true);
  });

  it('rejects a bare folder name', () => {
    // The exact value the old `webkitdirectory` picker stored, and the reason
    // every sync of that source failed with `folder does not exist`.
    expect(isAbsoluteFolderPath('AI Memory Hub')).toBe(false);
  });

  it('rejects relative paths', () => {
    expect(isAbsoluteFolderPath('Documents/Notes')).toBe(false);
    expect(isAbsoluteFolderPath('./Notes')).toBe(false);
    expect(isAbsoluteFolderPath('../Notes')).toBe(false);
  });

  it('rejects a tilde path, which the core does not expand', () => {
    // `PathBuf::from("~/Notes")` is taken literally, so this would look absolute
    // to a reader and fail exactly like the bare name did.
    expect(isAbsoluteFolderPath('~/Notes')).toBe(false);
    expect(isAbsoluteFolderPath('~')).toBe(false);
  });

  it('rejects empty and whitespace-only input', () => {
    expect(isAbsoluteFolderPath('')).toBe(false);
    expect(isAbsoluteFolderPath('   ')).toBe(false);
  });

  it('ignores surrounding whitespace on an otherwise valid path', () => {
    // Paths arrive pasted as often as picked.
    expect(isAbsoluteFolderPath('  /Users/alex/Notes  ')).toBe(true);
  });

  it('does not mistake a colon inside a name for a drive letter', () => {
    expect(isAbsoluteFolderPath('Notes: drafts')).toBe(false);
  });
});
