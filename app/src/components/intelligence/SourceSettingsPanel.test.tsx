import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { type MemorySourceEntry, updateMemorySource } from '../../services/memorySourcesService';
import { SourceSettingsPanel } from './SourceSettingsPanel';

const pickFolderViaDialog = vi.fn();
vi.mock('../../utils/tauriCommands/workspacePaths', () => ({
  pickFolderViaDialog: () => pickFolderViaDialog(),
}));
vi.mock('../../utils/tauriCommands/common', () => ({ isTauri: () => true }));

vi.mock('../../services/memorySourcesService', async () => {
  const actual = await vi.importActual<typeof import('../../services/memorySourcesService')>(
    '../../services/memorySourcesService'
  );
  return { ...actual, updateMemorySource: vi.fn() };
});

const source: MemorySourceEntry = {
  id: 'src-1',
  kind: 'rss_feed',
  label: 'My Feed',
  enabled: true,
  max_items: 50,
};

describe('<SourceSettingsPanel />', () => {
  afterEach(() => {
    vi.mocked(updateMemorySource).mockReset();
  });

  it('renders a labelled numeric field per relevant limit and a save button', () => {
    render(<SourceSettingsPanel source={source} onSaved={vi.fn()} />);
    const field = screen.getByLabelText(/Max Items/i) as HTMLInputElement;
    expect(field).toBeInTheDocument();
    expect(field.value).toBe('50');
    expect(screen.getByRole('button', { name: /Save/i })).toBeInTheDocument();
  });

  it('saves the edited value and reports the updated entry', async () => {
    const updated = { ...source, max_items: 75 };
    vi.mocked(updateMemorySource).mockResolvedValue(updated);
    const onSaved = vi.fn();
    const user = userEvent.setup();

    render(<SourceSettingsPanel source={source} onSaved={onSaved} />);
    const field = screen.getByLabelText(/Max Items/i);
    await user.clear(field);
    await user.type(field, '75');
    await user.click(screen.getByRole('button', { name: /Save/i }));

    expect(updateMemorySource).toHaveBeenCalledWith('src-1', { max_items: 75 });
    expect(onSaved).toHaveBeenCalledWith(updated);
  });

  it('rejects a negative value and reports the failure via onToast without saving', async () => {
    const onToast = vi.fn();
    const user = userEvent.setup();

    render(<SourceSettingsPanel source={source} onSaved={vi.fn()} onToast={onToast} />);
    const field = screen.getByLabelText(/Max Items/i);
    await user.clear(field);
    await user.type(field, '-5');
    await user.click(screen.getByRole('button', { name: /Save/i }));

    expect(updateMemorySource).not.toHaveBeenCalled();
    expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
  });
});

describe('<SourceSettingsPanel /> folder path', () => {
  const folder: MemorySourceEntry = {
    id: 'src-folder',
    kind: 'folder',
    label: 'AI Memory Hub',
    enabled: true,
    // The value the broken picker actually stored: a bare folder NAME.
    path: 'AI Memory Hub',
  };

  afterEach(() => {
    vi.mocked(updateMemorySource).mockReset();
    pickFolderViaDialog.mockReset();
  });

  it('exposes the stored path so a wrong one can be repaired in place', () => {
    render(<SourceSettingsPanel source={folder} onSaved={vi.fn()} />);

    // Without this field the only remedy for a mis-stored path was deleting the
    // source and re-adding it, losing whatever it had already synced.
    expect(screen.getByTestId('source-settings-path-src-folder')).toHaveValue('AI Memory Hub');
  });

  it('warns that the stored path is not absolute', () => {
    render(<SourceSettingsPanel source={folder} onSaved={vi.fn()} />);

    expect(screen.getByTestId('source-settings-path-hint-src-folder')).toBeInTheDocument();
  });

  it('refuses to save a path that is still not absolute', async () => {
    const onToast = vi.fn();
    render(<SourceSettingsPanel source={folder} onSaved={vi.fn()} onToast={onToast} />);

    await userEvent.clear(screen.getByTestId('source-settings-path-src-folder'));
    await userEvent.type(screen.getByTestId('source-settings-path-src-folder'), 'Other Folder');
    await userEvent.click(screen.getByRole('button', { name: /save/i }));

    expect(updateMemorySource).not.toHaveBeenCalled();
    expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
  });

  it('saves a corrected absolute path', async () => {
    vi.mocked(updateMemorySource).mockResolvedValue({ ...folder, path: '/Users/alex/Vault' });
    render(<SourceSettingsPanel source={folder} onSaved={vi.fn()} />);

    await userEvent.clear(screen.getByTestId('source-settings-path-src-folder'));
    await userEvent.type(
      screen.getByTestId('source-settings-path-src-folder'),
      '/Users/alex/Vault'
    );
    await userEvent.click(screen.getByRole('button', { name: /save/i }));

    expect(updateMemorySource).toHaveBeenCalledWith(
      'src-folder',
      expect.objectContaining({ path: '/Users/alex/Vault' })
    );
  });

  it('fills the field from the native picker', async () => {
    pickFolderViaDialog.mockResolvedValue('/Users/alex/AI Memory Hub');
    render(<SourceSettingsPanel source={folder} onSaved={vi.fn()} />);

    await userEvent.click(screen.getByTestId('source-settings-browse-src-folder'));

    expect(await screen.findByTestId('source-settings-path-src-folder')).toHaveValue(
      '/Users/alex/AI Memory Hub'
    );
  });

  it('keeps the typed path when the picker is cancelled', async () => {
    // `null` is a dismissed dialog and must not wipe what is already there.
    pickFolderViaDialog.mockResolvedValue(null);
    render(<SourceSettingsPanel source={folder} onSaved={vi.fn()} />);

    await userEvent.click(screen.getByTestId('source-settings-browse-src-folder'));

    expect(screen.getByTestId('source-settings-path-src-folder')).toHaveValue('AI Memory Hub');
  });

  it('leaves the path out of the patch when it was not touched', async () => {
    const untouched: MemorySourceEntry = { ...folder, path: '/Users/alex/Vault' };
    vi.mocked(updateMemorySource).mockResolvedValue(untouched);
    render(<SourceSettingsPanel source={untouched} onSaved={vi.fn()} />);

    await userEvent.click(screen.getByRole('button', { name: /save/i }));

    const patch = vi.mocked(updateMemorySource).mock.calls[0]?.[1] as Record<string, unknown>;
    expect(patch).not.toHaveProperty('path');
  });
});
