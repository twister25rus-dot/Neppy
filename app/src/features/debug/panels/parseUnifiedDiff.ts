/**
 * Pure parser for `git diff` unified output, as returned in `DebugDiff.text`.
 *
 * Splits the text into one entry per file with per-line classification so the
 * viewer can colour lines without re-scanning. Header lines (`index`, `---`,
 * `+++`, mode lines) are consumed for metadata and not emitted as body lines.
 */

export type DiffLineKind = 'add' | 'del' | 'context' | 'hunk' | 'note';

export interface DiffLine {
  kind: DiffLineKind;
  /** Line text without the leading `+` / `-` / space marker (hunk and note keep theirs). */
  text: string;
}

export type ParsedFileStatus = 'modified' | 'created' | 'deleted' | 'renamed';

export interface ParsedDiffFile {
  path: string;
  oldPath: string | null;
  status: ParsedFileStatus;
  binary: boolean;
  added: number;
  removed: number;
  lines: DiffLine[];
}

const HEADER_RE = /^diff --git a\/(.+) b\/(.+)$/;

function stripPrefix(raw: string, prefix: 'a/' | 'b/'): string | null {
  const value = raw.replace(/\t.*$/, '');
  if (value === '/dev/null') return null;
  return value.startsWith(prefix) ? value.slice(prefix.length) : value;
}

export function parseUnifiedDiff(text: string): ParsedDiffFile[] {
  const files: ParsedDiffFile[] = [];
  if (!text) return files;

  let current: ParsedDiffFile | null = null;
  let inHunk = false;

  for (const raw of text.split('\n')) {
    const header = HEADER_RE.exec(raw);
    if (header) {
      current = {
        path: header[2],
        oldPath: header[1] !== header[2] ? header[1] : null,
        status: 'modified',
        binary: false,
        added: 0,
        removed: 0,
        lines: [],
      };
      files.push(current);
      inHunk = false;
      continue;
    }
    if (!current) continue;

    if (!inHunk) {
      if (raw.startsWith('@@')) {
        inHunk = true;
        current.lines.push({ kind: 'hunk', text: raw });
      } else if (raw.startsWith('new file mode')) {
        current.status = 'created';
      } else if (raw.startsWith('deleted file mode')) {
        current.status = 'deleted';
      } else if (raw.startsWith('rename from ') || raw.startsWith('rename to ')) {
        current.status = 'renamed';
      } else if (raw.startsWith('Binary files ') || raw.startsWith('GIT binary patch')) {
        current.binary = true;
      } else if (raw.startsWith('--- ')) {
        const old = stripPrefix(raw.slice(4), 'a/');
        if (old === null) current.status = 'created';
        else if (current.status === 'deleted') current.path = old;
      } else if (raw.startsWith('+++ ')) {
        const next = stripPrefix(raw.slice(4), 'b/');
        if (next === null) current.status = 'deleted';
        else current.path = next;
      }
      continue;
    }

    if (raw.startsWith('@@')) {
      current.lines.push({ kind: 'hunk', text: raw });
    } else if (raw.startsWith('\\')) {
      current.lines.push({ kind: 'note', text: raw });
    } else if (raw.startsWith('+')) {
      current.added += 1;
      current.lines.push({ kind: 'add', text: raw.slice(1) });
    } else if (raw.startsWith('-')) {
      current.removed += 1;
      current.lines.push({ kind: 'del', text: raw.slice(1) });
    } else if (raw.startsWith(' ')) {
      current.lines.push({ kind: 'context', text: raw.slice(1) });
    }
    // Anything else in a hunk is the trailing empty string from the final newline.
  }

  return files;
}
