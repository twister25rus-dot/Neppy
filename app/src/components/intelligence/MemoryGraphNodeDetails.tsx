/**
 * Side sheet opened by clicking a node in the Brain memory graph.
 *
 * Shows what the node *is* (kind, level, source, time range, id), what it is
 * connected to (parent + children in tree mode, mention edges in contacts
 * mode, each clickable to hop along the graph), and its stored text: the
 * sealed summary file for a summary node, the vault body for a chunk. See
 * {@link loadGraphNodeContent} for where each body comes from.
 *
 * Built on the shared Radix-backed {@link SheetRoot} (focus trap, Escape to
 * close, focus restore), so it adds no hand-rolled overlay.
 */
import { useEffect, useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { GraphEdge, GraphMode, GraphNode } from '../../utils/tauriCommands';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import { ErrorBanner, Spinner } from '../ui/LoadingState';
import { SheetContent, SheetDescription, SheetRoot, SheetTitle } from '../ui/Sheet';
import { nodeColor } from './memoryGraphLayout';
import {
  type GraphNodeContent,
  loadGraphNodeContent,
  nodeHasLoadableContent,
} from './memoryGraphNodeContent';

/** How many connected nodes are listed before collapsing into "+N more". */
const CONNECTED_LIMIT = 12;

interface MemoryGraphNodeDetailsProps {
  /** The open node; `null` keeps the sheet closed. */
  node: GraphNode | null;
  /** The whole graph, used to resolve parent / children / mention neighbours. */
  nodes: GraphNode[];
  edges: GraphEdge[];
  mode: GraphMode;
  onClose: () => void;
  /** Hop to a connected node (re-targets the sheet). */
  onSelectNode: (node: GraphNode) => void;
  /** Open a workspace-relative file (summary `.md` / chunk body) externally. */
  onOpenFile: (workspacePath: string) => void;
}

type LoadState =
  | { key: string | null; status: 'idle' }
  | { key: string; status: 'loading' }
  | { key: string; status: 'ready'; content: GraphNodeContent | null }
  | { key: string; status: 'error'; error: string };

function initialState(node: GraphNode | null): LoadState {
  if (!node) return { key: null, status: 'idle' };
  return nodeHasLoadableContent(node)
    ? { key: node.id, status: 'loading' }
    : { key: node.id, status: 'idle' };
}

function formatMs(ms?: number): string | null {
  if (typeof ms !== 'number' || !Number.isFinite(ms)) return null;
  const date = new Date(ms);
  return Number.isNaN(date.getTime()) ? null : date.toLocaleString();
}

function formatRange(start?: number, end?: number): string | null {
  const a = formatMs(start);
  const b = formatMs(end);
  if (!a && !b) return null;
  if (!a || !b || a === b) return a ?? b;
  return `${a} → ${b}`;
}

/** Neighbours of `node`: its parent first, then children / mention peers. */
function connectedNodes(
  node: GraphNode,
  nodes: GraphNode[],
  edges: GraphEdge[],
  mode: GraphMode
): GraphNode[] {
  const byId = new Map(nodes.map(n => [n.id, n]));
  const out: GraphNode[] = [];
  const seen = new Set<string>([node.id]);
  const push = (n: GraphNode | undefined) => {
    if (!n || seen.has(n.id)) return;
    seen.add(n.id);
    out.push(n);
  };
  if (mode === 'tree') {
    if (node.parent_id) push(byId.get(node.parent_id));
    for (const n of nodes) if (n.parent_id === node.id) push(n);
  } else {
    for (const e of edges) {
      if (e.from === node.id) push(byId.get(e.to));
      else if (e.to === node.id) push(byId.get(e.from));
    }
  }
  return out;
}

export function MemoryGraphNodeDetails({
  node,
  nodes,
  edges,
  mode,
  onClose,
  onSelectNode,
  onOpenFile,
}: MemoryGraphNodeDetailsProps) {
  const { t } = useT();
  const [load, setLoad] = useState<LoadState>(() => initialState(node));

  // Reset during render when the target node changes (not in an effect), so
  // the sheet never flashes the previous node's body.
  const nodeKey = node?.id ?? null;
  if (load.key !== nodeKey) setLoad(initialState(node));

  useEffect(() => {
    if (!node || !nodeHasLoadableContent(node)) return;
    let cancelled = false;
    const id = node.id;
    console.debug('[memory-graph-details] load entry kind=%s id=%s', node.kind, id);
    loadGraphNodeContent(node)
      .then(content => {
        if (cancelled) return;
        console.debug(
          '[memory-graph-details] load exit id=%s has_text=%s',
          id,
          !!content && content.text.length > 0
        );
        setLoad({ key: id, status: 'ready', content });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        console.error('[memory-graph-details] load failed', err);
        setLoad({
          key: id,
          status: 'error',
          error: err instanceof Error ? err.message : String(err),
        });
      });
    return () => {
      cancelled = true;
    };
  }, [node]);

  const connected = useMemo(
    () => (node ? connectedNodes(node, nodes, edges, mode) : []),
    [node, nodes, edges, mode]
  );

  if (!node) return null;

  const kindLabel =
    node.kind === 'summary'
      ? `L${node.level ?? '?'} ${t('graph.tooltip.summary')}`
      : node.kind === 'source'
        ? t('graph.source')
        : node.kind === 'contact'
          ? t('graph.contact')
          : node.kind === 'root'
            ? t('graph.details.rootKind')
            : t('graph.document');

  const content = load.status === 'ready' ? load.content : null;
  const parent = node.parent_id ? nodes.find(n => n.id === node.parent_id) : undefined;
  // A chunk hanging straight off its source root has not been sealed into a
  // summary yet — the export only does that for leaves with no summary.
  const unsealed = mode === 'tree' && node.kind === 'chunk' && parent?.kind === 'source';
  const timeRange =
    formatRange(node.time_range_start_ms, node.time_range_end_ms) ?? formatMs(content?.timestampMs);
  const sourceLabel =
    node.kind === 'source'
      ? null
      : ((parent?.kind === 'source' ? parent.label : null) ??
        node.tree_scope ??
        content?.sourceId ??
        null);

  const meta: Array<{ label: string; value: string; mono?: boolean }> = [
    { label: t('graph.details.type'), value: kindLabel },
  ];
  if (node.kind === 'summary' && node.tree_kind) {
    meta.push({ label: t('graph.details.tree'), value: node.tree_kind });
  }
  if (sourceLabel) meta.push({ label: t('graph.source'), value: sourceLabel });
  if (node.kind === 'summary' && typeof node.child_count === 'number') {
    meta.push({ label: t('graph.details.childCount'), value: String(node.child_count) });
  }
  if (timeRange) meta.push({ label: t('graph.details.timeRange'), value: timeRange });
  if (typeof content?.tokenCount === 'number') {
    meta.push({ label: t('graph.details.tokens'), value: String(content.tokenCount) });
  }
  meta.push({ label: t('graph.details.id'), value: node.id, mono: true });

  const shown = connected.slice(0, CONNECTED_LIMIT);
  const hidden = connected.length - shown.length;
  const filePath = content?.workspacePath ?? null;

  return (
    <SheetRoot
      open
      onOpenChange={next => {
        if (!next) {
          console.debug('[memory-graph-details] close id=%s', node.id);
          onClose();
        }
      }}>
      <SheetContent side="right" className="max-w-lg" data-testid="memory-graph-node-details">
        <header className="flex shrink-0 items-start gap-2.5 border-b border-line px-4 py-3">
          <span
            aria-hidden
            className="mt-1.5 inline-block h-2.5 w-2.5 shrink-0 rounded-full"
            style={{ backgroundColor: nodeColor(node) }}
          />
          <div className="min-w-0 flex-1">
            <SheetTitle className="break-words text-sm font-semibold text-content">
              {node.label || node.id}
            </SheetTitle>
            <SheetDescription asChild>
              <div className="mt-1 flex flex-wrap items-center gap-1.5">
                <Badge data-testid="memory-graph-node-details-kind">{kindLabel}</Badge>
                {unsealed && (
                  <Badge variant="warning" data-testid="memory-graph-node-details-unsealed">
                    {t('graph.details.unsealed')}
                  </Badge>
                )}
              </div>
            </SheetDescription>
          </div>
          <Button
            variant="tertiary"
            size="xs"
            iconOnly
            aria-label={t('graph.details.close')}
            data-testid="memory-graph-node-details-close"
            onClick={onClose}>
            ✕
          </Button>
        </header>

        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-4 py-4">
          <dl className="grid grid-cols-[auto,1fr] gap-x-4 gap-y-1.5 text-xs">
            {meta.map(row => (
              <div key={row.label} className="contents">
                <dt className="text-content-muted">{row.label}</dt>
                <dd
                  className={`min-w-0 break-words text-content-secondary ${row.mono ? 'font-mono text-[11px]' : ''}`}>
                  {row.value}
                </dd>
              </div>
            ))}
          </dl>

          {nodeHasLoadableContent(node) && (
            <section>
              <div className="mb-2 flex items-center justify-between gap-2">
                <h3 className="text-xs font-medium uppercase tracking-wide text-content-muted">
                  {t('graph.details.content')}
                </h3>
                {filePath && (
                  <Button
                    variant="secondary"
                    size="xs"
                    data-testid="memory-graph-node-details-open-file"
                    onClick={() => onOpenFile(filePath)}>
                    {t('graph.details.openFile')}
                  </Button>
                )}
              </div>
              {load.status === 'loading' ? (
                <div
                  className="flex items-center gap-2 text-xs text-content-muted"
                  data-testid="memory-graph-node-details-loading">
                  <Spinner className="h-3 w-3" />
                  {t('graph.details.loading')}
                </div>
              ) : load.status === 'error' ? (
                <ErrorBanner>
                  {t('graph.details.loadFailed')}
                  <span className="mt-1 block break-words font-mono text-[11px]">{load.error}</span>
                </ErrorBanner>
              ) : content && content.text.trim().length > 0 ? (
                <>
                  <pre
                    className="max-h-[55vh] overflow-auto whitespace-pre-wrap break-words rounded-md bg-surface-muted p-3 text-xs leading-relaxed text-content-secondary"
                    data-testid="memory-graph-node-details-content">
                    {content.text}
                  </pre>
                  {content.truncated && (
                    <p className="mt-1.5 text-[11px] text-content-faint">
                      {t('graph.details.truncated')}
                    </p>
                  )}
                </>
              ) : (
                <p
                  className="text-xs text-content-muted"
                  data-testid="memory-graph-node-details-empty">
                  {t('graph.details.noContent')}
                </p>
              )}
            </section>
          )}

          <section>
            <h3 className="mb-2 text-xs font-medium uppercase tracking-wide text-content-muted">
              {t('graph.details.connected')} ({connected.length})
            </h3>
            {connected.length === 0 ? (
              <p
                className="text-xs text-content-muted"
                data-testid="memory-graph-node-details-no-connections">
                {t('graph.details.noConnections')}
              </p>
            ) : (
              <ul className="space-y-1" data-testid="memory-graph-node-details-connected">
                {shown.map(n => (
                  <li key={n.id}>
                    <button
                      type="button"
                      className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-xs text-content-secondary hover:bg-surface-muted"
                      data-testid={`memory-graph-node-details-link-${n.id}`}
                      onClick={() => {
                        console.debug('[memory-graph-details] hop %s -> %s', node.id, n.id);
                        onSelectNode(n);
                      }}>
                      <span
                        aria-hidden
                        className="inline-block h-2 w-2 shrink-0 rounded-full"
                        style={{ backgroundColor: nodeColor(n) }}
                      />
                      <span className="min-w-0 flex-1 truncate">{n.label || n.id}</span>
                      {n.id === node.parent_id && (
                        <span className="shrink-0 text-[10px] text-content-faint">
                          {t('graph.details.parent')}
                        </span>
                      )}
                    </button>
                  </li>
                ))}
                {hidden > 0 && (
                  <li className="px-2 text-[11px] text-content-faint">
                    {t('graph.details.more').replace('{count}', String(hidden))}
                  </li>
                )}
              </ul>
            )}
          </section>
        </div>
      </SheetContent>
    </SheetRoot>
  );
}
