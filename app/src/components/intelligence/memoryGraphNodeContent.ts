/**
 * Content loader for the Brain graph's node detail sheet.
 *
 * The graph export carries structure (ids, labels, parent links, time ranges)
 * but not bodies, so opening a node fetches its text on demand:
 *
 *   - `summary` → the sealed summary's `.md` file in the workspace content
 *     vault, through the shared guarded `preview_workspace_text` command (the
 *     same path the hover "Preview" button uses).
 *   - `chunk`   → `openhuman.memory_tree_get_chunk`, preferring the vault
 *     `body` (full note) over `chunk.content` (the stored ≤500-char preview).
 *     Synthetic document leaves (`doc:<scope>:<child>`) are not chunks and
 *     have nothing to fetch.
 *   - `source` / `contact` / `root` → no body; the sheet shows metadata and
 *     connected nodes only.
 *
 * Logging: `[memory-graph-details]` prefix. Never logs bodies or paths that
 * embed the user's home directory.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import type { GraphNode } from '../../utils/tauriCommands';
import { previewWorkspaceText } from '../../utils/tauriCommands/workspacePaths';
import { MEMORY_CONTENT_WORKSPACE_PATH, summaryWorkspacePath } from './memoryWorkspacePaths';

export interface GraphNodeContent {
  /** The text to show. Empty string when the store holds none. */
  text: string;
  /** True when `text` is a cut-down preview rather than the full body. */
  truncated: boolean;
  /** Workspace-relative path of the body file, when it has one. */
  workspacePath: string | null;
  /** Chunk-only metadata from the store. */
  tokenCount?: number;
  sourceId?: string;
  timestampMs?: number;
}

/** Wire shape of the chunk returned by `memory_tree_get_chunk` (tinymemory-bus `Chunk`). */
interface WireChunk {
  id: string;
  content: string;
  token_count?: number;
  metadata?: { source_id?: string; timestamp?: number };
}

interface GetChunkResponse {
  chunk: WireChunk | null;
  body?: string | null;
  content_path?: string | null;
}

/** Only real chunk ids can be fetched; `doc:` leaves are synthesised by the export. */
export function isFetchableChunk(node: GraphNode): boolean {
  return node.kind === 'chunk' && !node.id.startsWith('doc:');
}

/** Whether opening this node triggers a content fetch at all. */
export function nodeHasLoadableContent(node: GraphNode): boolean {
  if (node.kind === 'summary') return summaryWorkspacePath(node) != null;
  return isFetchableChunk(node);
}

function unwrap<T>(resp: T | { result?: T }): T {
  if (resp && typeof resp === 'object' && 'result' in (resp as Record<string, unknown>)) {
    return (resp as { result: T }).result;
  }
  return resp as T;
}

async function loadChunk(node: GraphNode): Promise<GraphNodeContent> {
  console.debug('[memory-graph-details] get_chunk entry id=%s', node.id);
  const raw = await callCoreRpc<GetChunkResponse | { result?: GetChunkResponse }>({
    method: 'openhuman.memory_tree_get_chunk',
    params: { id: node.id },
  });
  const resp = unwrap<GetChunkResponse>(raw);
  const chunk = resp?.chunk ?? null;
  if (!chunk) {
    console.debug('[memory-graph-details] get_chunk exit id=%s found=false', node.id);
    return { text: '', truncated: false, workspacePath: null };
  }
  const body = typeof resp.body === 'string' ? resp.body : null;
  const text = body ?? chunk.content ?? '';
  const contentPath = resp.content_path
    ? `${MEMORY_CONTENT_WORKSPACE_PATH}/${resp.content_path.replace(/^\/+/, '')}`
    : null;
  console.debug(
    '[memory-graph-details] get_chunk exit id=%s found=true body=%s chars=%d',
    node.id,
    body != null,
    text.length
  );
  return {
    text,
    // Without a vault body we are showing the stored preview; say so when the
    // store says the chunk is larger than what came back.
    truncated: body == null && !!contentPath,
    workspacePath: contentPath,
    tokenCount: chunk.token_count,
    sourceId: chunk.metadata?.source_id,
    timestampMs: chunk.metadata?.timestamp,
  };
}

async function loadSummary(node: GraphNode): Promise<GraphNodeContent> {
  const path = summaryWorkspacePath(node);
  if (!path) return { text: '', truncated: false, workspacePath: null };
  console.debug('[memory-graph-details] summary preview entry id=%s', node.id);
  const preview = await previewWorkspaceText(path);
  console.debug(
    '[memory-graph-details] summary preview exit id=%s chars=%d truncated=%s',
    node.id,
    preview.contents.length,
    preview.truncated
  );
  return { text: preview.contents, truncated: preview.truncated, workspacePath: path };
}

/**
 * Fetch the displayable content for a graph node, or `null` when the node
 * kind has no stored body (source roots, contacts, synthetic document leaves).
 */
export async function loadGraphNodeContent(node: GraphNode): Promise<GraphNodeContent | null> {
  if (node.kind === 'summary') return loadSummary(node);
  if (isFetchableChunk(node)) return loadChunk(node);
  console.debug('[memory-graph-details] no content to load kind=%s', node.kind);
  return null;
}
