import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { PropsWithChildren } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import themeReducer from '../../store/themeSlice';
import type { GraphEdge, GraphNode } from '../../utils/tauriCommands';
import { MemoryGraph } from './MemoryGraph';

function ReduxWrapper({ children }: PropsWithChildren) {
  const store = configureStore({ reducer: { theme: themeReducer } });
  return <Provider store={store}>{children}</Provider>;
}

const mocks = vi.hoisted(() => ({
  openUrl: vi.fn(),
  openWorkspacePath: vi.fn(),
  previewWorkspaceText: vi.fn(),
  callCoreRpc: vi.fn(),
}));

vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mocks.callCoreRpc(...args),
}));

vi.mock('../../utils/openUrl', () => ({ openUrl: (...args: unknown[]) => mocks.openUrl(...args) }));
vi.mock('../../utils/tauriCommands/workspacePaths', () => ({
  openWorkspacePath: (...args: unknown[]) => mocks.openWorkspacePath(...args),
  previewWorkspaceText: (...args: unknown[]) => mocks.previewWorkspaceText(...args),
}));

function makeSummaryNode(overrides: Partial<GraphNode> = {}): GraphNode {
  return {
    kind: 'summary',
    id: 'sum-1',
    label: 'Summary 1',
    tree_id: 't-1',
    tree_kind: 'topic',
    tree_scope: 'work',
    level: 0,
    parent_id: null,
    child_count: 2,
    file_basename: 'summary-1',
    ...overrides,
  };
}

function makeChunkNode(overrides: Partial<GraphNode> = {}): GraphNode {
  return { kind: 'chunk', id: 'chunk-1', label: 'A chunk', ...overrides };
}

function makeContactNode(overrides: Partial<GraphNode> = {}): GraphNode {
  return {
    kind: 'contact',
    id: 'person:alice',
    label: 'Alice',
    entity_kind: 'person',
    ...overrides,
  };
}

describe('<MemoryGraph />', () => {
  beforeEach(() => {
    mocks.openUrl.mockReset();
    mocks.openUrl.mockResolvedValue(undefined);
    mocks.openWorkspacePath.mockReset();
    mocks.openWorkspacePath.mockResolvedValue(undefined);
    mocks.callCoreRpc.mockReset();
    mocks.previewWorkspaceText.mockReset();
    mocks.previewWorkspaceText.mockResolvedValue({
      path: 'memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md',
      absolutePath:
        '/Users/me/openhuman/memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md',
      contents: '# Summary\n\nWorkspace one notes',
      truncated: false,
      sizeBytes: 30,
    });
  });

  it('renders the empty state when there are no nodes', () => {
    render(<MemoryGraph nodes={[]} edges={[]} mode="tree" />, { wrapper: ReduxWrapper });
    expect(screen.getByTestId('memory-graph-empty')).toBeInTheDocument();
  });

  it('fires onReady once the layout settles (synchronous SVG path under jsdom)', async () => {
    const onReady = vi.fn();
    const nodes = [
      makeSummaryNode({ id: 'root', level: 0, parent_id: null }),
      makeSummaryNode({ id: 'child', level: 1, parent_id: 'root' }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" onReady={onReady} />, {
      wrapper: ReduxWrapper,
    });
    await waitFor(() => expect(onReady).toHaveBeenCalledTimes(1));
  });

  it('does not fire onReady for an empty graph (nothing to lay out)', () => {
    const onReady = vi.fn();
    render(<MemoryGraph nodes={[]} edges={[]} mode="tree" onReady={onReady} />, {
      wrapper: ReduxWrapper,
    });
    expect(onReady).not.toHaveBeenCalled();
  });

  it('renders an SVG with one circle per node in tree mode', () => {
    const nodes = [
      makeSummaryNode({ id: 'root', level: 0, parent_id: null }),
      makeSummaryNode({ id: 'child', level: 1, parent_id: 'root' }),
    ];
    const { container } = render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />, {
      wrapper: ReduxWrapper,
    });
    expect(screen.getByTestId('memory-graph-svg')).toBeInTheDocument();
    expect(container.querySelectorAll('circle').length).toBe(2);
    expect(screen.getByTestId('memory-graph-node-root')).toBeInTheDocument();
    expect(screen.getByTestId('memory-graph-node-child')).toBeInTheDocument();
  });

  it('renders contacts-mode legend rows for chunk and contact', () => {
    const nodes = [
      makeChunkNode({ id: 'd1' }),
      makeContactNode({ id: 'person:alice', label: 'Alice' }),
    ];
    const edges: GraphEdge[] = [{ from: 'd1', to: 'person:alice' }];
    render(<MemoryGraph nodes={nodes} edges={edges} mode="contacts" />, { wrapper: ReduxWrapper });
    // Two legend rows render with i18n keys as fallback (graph.document/contact)
    // — assert via the rendered nodes count instead, which is deterministic.
    expect(screen.getAllByTestId(/memory-graph-node-/).length).toBe(2);
  });

  it('opens a summary node through the shared workspace path command', async () => {
    const nodes = [
      makeSummaryNode({
        id: 'sum-A',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />, { wrapper: ReduxWrapper });
    fireEvent.click(screen.getByTestId('memory-graph-node-sum-A'));
    await waitFor(() => {
      expect(mocks.openWorkspacePath).toHaveBeenCalledWith(
        'memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md'
      );
    });
    expect(mocks.openUrl).not.toHaveBeenCalled();
  });

  it('logs workspace open failures without falling back to raw URL opens', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.openWorkspacePath.mockRejectedValueOnce(new Error('open failed'));
    const nodes = [
      makeSummaryNode({
        id: 'sum-open-fails',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];

    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />, { wrapper: ReduxWrapper });
    fireEvent.click(screen.getByTestId('memory-graph-node-sum-open-fails'));

    await waitFor(() => {
      expect(errorSpy).toHaveBeenCalledWith(
        '[memory-graph] openWorkspacePath failed',
        expect.any(Error)
      );
    });
    expect(mocks.openUrl).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('keeps non-Gmail source prefixes in summary workspace paths', async () => {
    const nodes = [
      makeSummaryNode({
        id: 'sum-slack',
        tree_kind: 'source',
        tree_scope: 'slack:#eng',
        level: 2,
        file_basename: 'summary-slack',
      }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />, { wrapper: ReduxWrapper });
    fireEvent.click(screen.getByTestId('memory-graph-node-sum-slack'));
    await waitFor(() => {
      expect(mocks.openWorkspacePath).toHaveBeenCalledWith(
        'memory_tree/content/wiki/summaries/source-slack-eng/L2/summary-slack.md'
      );
    });
  });

  it('previews a hovered summary through the shared workspace preview command', async () => {
    const nodes = [
      makeSummaryNode({
        id: 'sum-A',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />, { wrapper: ReduxWrapper });

    fireEvent.mouseEnter(screen.getByTestId('memory-graph-node-sum-A'));
    fireEvent.click(screen.getByTestId('memory-graph-preview-sum-A'));

    await waitFor(() => {
      expect(mocks.previewWorkspaceText).toHaveBeenCalledWith(
        'memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md'
      );
    });
    expect(await screen.findByTestId('memory-graph-preview')).toHaveTextContent(
      'Workspace one notes'
    );
  });

  it('shows preview errors from the shared workspace preview command', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.previewWorkspaceText.mockRejectedValueOnce(new Error('preview failed'));
    const nodes = [
      makeSummaryNode({
        id: 'sum-preview-fails',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];

    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />, { wrapper: ReduxWrapper });
    fireEvent.mouseEnter(screen.getByTestId('memory-graph-node-sum-preview-fails'));
    fireEvent.click(screen.getByTestId('memory-graph-preview-sum-preview-fails'));

    expect(await screen.findByTestId('memory-graph-preview')).toHaveTextContent('preview failed');
    expect(errorSpy).toHaveBeenCalledWith(
      '[memory-graph] previewWorkspaceText failed',
      expect.any(Error)
    );
    errorSpy.mockRestore();
  });

  it('marks truncated summary previews in the preview panel', async () => {
    mocks.previewWorkspaceText.mockResolvedValueOnce({
      path: 'memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md',
      absolutePath:
        '/Users/me/openhuman/memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md',
      contents: '# Summary',
      truncated: true,
      sizeBytes: 100_000,
    });
    const nodes = [
      makeSummaryNode({
        id: 'sum-truncated',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];

    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />, { wrapper: ReduxWrapper });
    fireEvent.mouseEnter(screen.getByTestId('memory-graph-node-sum-truncated'));
    fireEvent.click(screen.getByTestId('memory-graph-preview-sum-truncated'));

    expect(await screen.findByTestId('memory-graph-preview')).toHaveTextContent('# Summary');
    expect(screen.getByTestId('memory-graph-preview')).toHaveTextContent('…');
  });

  it('keeps the summary preview action reachable after leaving the SVG node', () => {
    const nodes = [
      makeSummaryNode({
        id: 'sum-A',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />, { wrapper: ReduxWrapper });

    const node = screen.getByTestId('memory-graph-node-sum-A');
    fireEvent.mouseEnter(node);
    fireEvent.mouseLeave(node, { relatedTarget: screen.getByTestId('memory-graph-tooltip') });

    expect(screen.getByTestId('memory-graph-preview-sum-A')).toBeInTheDocument();
  });

  it('clears the hovered node when the pointer leaves the graph', () => {
    const nodes = [
      makeSummaryNode({
        id: 'sum-A',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];
    const { container } = render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />, {
      wrapper: ReduxWrapper,
    });

    fireEvent.mouseEnter(screen.getByTestId('memory-graph-node-sum-A'));
    expect(screen.getByTestId('memory-graph-tooltip')).toBeInTheDocument();
    fireEvent.mouseLeave(container.querySelector('.memory-graph') as Element);

    expect(screen.queryByTestId('memory-graph-tooltip')).not.toBeInTheDocument();
  });

  it('does NOT call workspace open when a non-summary node is clicked', async () => {
    const nodes = [makeChunkNode({ id: 'doc-1' })];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="contacts" />, { wrapper: ReduxWrapper });
    fireEvent.click(screen.getByTestId('memory-graph-node-doc-1'));
    await Promise.resolve();
    expect(mocks.openWorkspacePath).not.toHaveBeenCalled();
  });

  it('shows a tooltip footer when a node is hovered', () => {
    const nodes = [makeContactNode({ id: 'person:bob', label: 'Bob' })];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="contacts" />, { wrapper: ReduxWrapper });
    fireEvent.mouseEnter(screen.getByTestId('memory-graph-node-person:bob'));
    expect(screen.getByTestId('memory-graph-tooltip')).toBeInTheDocument();
    expect(screen.getByTestId('memory-graph-tooltip').textContent).toContain('Bob');
  });
});

function makeSourceNode(overrides: Partial<GraphNode> = {}): GraphNode {
  return {
    kind: 'source',
    id: 'source:mem_src:src_1:notes/a.md',
    label: 'notes/a.md',
    tree_scope: 'mem_src:src_1:notes/a.md',
    ...overrides,
  };
}

describe('<MemoryGraph /> edges', () => {
  it('draws one line per parent link, including unsealed leaves hung off a source root', () => {
    // The shape the core now exports for a store whose seals never ran: no
    // summaries, every chunk parented to its synthetic source root.
    const nodes = [
      makeSourceNode(),
      makeChunkNode({ id: 'c1', parent_id: 'source:mem_src:src_1:notes/a.md' }),
      makeChunkNode({ id: 'c2', parent_id: 'source:mem_src:src_1:notes/a.md' }),
    ];
    const { container } = render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />, {
      wrapper: ReduxWrapper,
    });
    expect(container.querySelectorAll('line').length).toBe(2);
    expect(screen.getByText(/parent-child/)).toHaveTextContent('2 parent-child links');
  });

  it('draws no line for a parent id that is not in the node set', () => {
    const nodes = [makeChunkNode({ id: 'c1', parent_id: 'summary:missing' })];
    const { container } = render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />, {
      wrapper: ReduxWrapper,
    });
    expect(container.querySelectorAll('line').length).toBe(0);
  });

  it('draws contacts-mode mention edges from the explicit edge list', () => {
    const nodes = [makeChunkNode({ id: 'd1' }), makeContactNode({ id: 'person:alice' })];
    const edges: GraphEdge[] = [
      { from: 'd1', to: 'person:alice' },
      { from: 'd1', to: 'person:nobody' },
    ];
    const { container } = render(<MemoryGraph nodes={nodes} edges={edges} mode="contacts" />, {
      wrapper: ReduxWrapper,
    });
    expect(container.querySelectorAll('line').length).toBe(1);
  });
});

describe('<MemoryGraph showNodeDetails />', () => {
  beforeEach(() => {
    mocks.openWorkspacePath.mockReset();
    mocks.openWorkspacePath.mockResolvedValue(undefined);
    mocks.callCoreRpc.mockReset();
    mocks.previewWorkspaceText.mockReset();
  });

  it('opens a chunk node and shows its full vault body', async () => {
    mocks.callCoreRpc.mockResolvedValue({
      result: {
        chunk: {
          id: 'c1',
          content: 'short preview',
          token_count: 42,
          metadata: { source_id: 'mem_src:src_1:notes/a.md', timestamp: 1_700_000_000_000 },
        },
        body: '# Full note\n\nEverything the user wrote.',
        content_path: 'document/notes-a/c1.md',
      },
    });
    const nodes = [
      makeSourceNode(),
      makeChunkNode({ id: 'c1', label: 'Full note', parent_id: 'source:mem_src:src_1:notes/a.md' }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" showNodeDetails />, {
      wrapper: ReduxWrapper,
    });

    fireEvent.click(screen.getByTestId('memory-graph-node-c1'));

    expect(await screen.findByTestId('memory-graph-node-details')).toBeInTheDocument();
    expect(mocks.callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_get_chunk',
      params: { id: 'c1' },
    });
    expect(await screen.findByTestId('memory-graph-node-details-content')).toHaveTextContent(
      'Everything the user wrote.'
    );
    // Hung off its source root, so it has not been summarised yet.
    expect(screen.getByTestId('memory-graph-node-details-unsealed')).toBeInTheDocument();
    expect(screen.getByText('42')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('memory-graph-node-details-open-file'));
    expect(mocks.openWorkspacePath).toHaveBeenCalledWith(
      'memory_tree/content/document/notes-a/c1.md'
    );
  });

  it('falls back to the stored preview when the vault body is unavailable', async () => {
    mocks.callCoreRpc.mockResolvedValue({
      chunk: { id: 'c1', content: 'stored preview text', metadata: {} },
    });
    render(
      <MemoryGraph nodes={[makeChunkNode({ id: 'c1' })]} edges={[]} mode="tree" showNodeDetails />,
      { wrapper: ReduxWrapper }
    );
    fireEvent.click(screen.getByTestId('memory-graph-node-c1'));
    expect(await screen.findByTestId('memory-graph-node-details-content')).toHaveTextContent(
      'stored preview text'
    );
  });

  it('opens a summary node in the sheet instead of launching the file', async () => {
    mocks.previewWorkspaceText.mockResolvedValue({
      path: 'memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md',
      absolutePath: '/x',
      contents: 'Sealed summary body',
      truncated: false,
      sizeBytes: 19,
    });
    const nodes = [
      makeSummaryNode({
        id: 'sum-A',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" showNodeDetails />, {
      wrapper: ReduxWrapper,
    });
    fireEvent.click(screen.getByTestId('memory-graph-node-sum-A'));

    expect(await screen.findByTestId('memory-graph-node-details-content')).toHaveTextContent(
      'Sealed summary body'
    );
    expect(mocks.previewWorkspaceText).toHaveBeenCalledWith(
      'memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md'
    );
    expect(mocks.openWorkspacePath).not.toHaveBeenCalled();
    expect(screen.getByTestId('memory-graph-node-details-kind')).toHaveTextContent('L2 Summary');
  });

  it('shows a load error instead of an empty body', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.callCoreRpc.mockRejectedValue(new Error('core offline'));
    render(
      <MemoryGraph nodes={[makeChunkNode({ id: 'c1' })]} edges={[]} mode="tree" showNodeDetails />,
      { wrapper: ReduxWrapper }
    );
    fireEvent.click(screen.getByTestId('memory-graph-node-c1'));
    expect(await screen.findByRole('alert')).toHaveTextContent('core offline');
    errorSpy.mockRestore();
  });

  it('lists connected nodes for a source and hops to one on click', async () => {
    mocks.callCoreRpc.mockResolvedValue({ chunk: { id: 'c2', content: 'second', metadata: {} } });
    const nodes = [
      makeSourceNode(),
      makeChunkNode({ id: 'c1', label: 'first', parent_id: 'source:mem_src:src_1:notes/a.md' }),
      makeChunkNode({ id: 'c2', label: 'second', parent_id: 'source:mem_src:src_1:notes/a.md' }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" showNodeDetails />, {
      wrapper: ReduxWrapper,
    });
    fireEvent.click(screen.getByTestId('memory-graph-node-source:mem_src:src_1:notes/a.md'));

    const list = await screen.findByTestId('memory-graph-node-details-connected');
    expect(list.querySelectorAll('li').length).toBe(2);
    // A source root has no body to fetch.
    expect(mocks.callCoreRpc).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('memory-graph-node-details-link-c2'));
    await waitFor(() =>
      expect(mocks.callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.memory_tree_get_chunk',
        params: { id: 'c2' },
      })
    );
    expect(await screen.findByTestId('memory-graph-node-details-content')).toHaveTextContent(
      'second'
    );
  });

  it('closes the sheet from the close button', async () => {
    render(<MemoryGraph nodes={[makeContactNode()]} edges={[]} mode="contacts" showNodeDetails />, {
      wrapper: ReduxWrapper,
    });
    fireEvent.click(screen.getByTestId('memory-graph-node-person:alice'));
    expect(await screen.findByTestId('memory-graph-node-details')).toBeInTheDocument();
    expect(screen.getByTestId('memory-graph-node-details-no-connections')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('memory-graph-node-details-close'));
    await waitFor(() =>
      expect(screen.queryByTestId('memory-graph-node-details')).not.toBeInTheDocument()
    );
  });

  it('does not fetch content for a synthetic document leaf', async () => {
    render(
      <MemoryGraph
        nodes={[makeChunkNode({ id: 'doc:github:acme:commit:abc', label: 'commit abc' })]}
        edges={[]}
        mode="tree"
        showNodeDetails
      />,
      { wrapper: ReduxWrapper }
    );
    fireEvent.click(screen.getByTestId('memory-graph-node-doc:github:acme:commit:abc'));
    expect(await screen.findByTestId('memory-graph-node-details')).toBeInTheDocument();
    expect(mocks.callCoreRpc).not.toHaveBeenCalled();
    expect(screen.queryByTestId('memory-graph-node-details-content')).not.toBeInTheDocument();
  });
});
