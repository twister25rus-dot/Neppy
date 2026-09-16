import { invoke } from '@tauri-apps/api/core';

import { isTauri } from './common';

interface RawWorkspaceTextPreview {
  path: string;
  absolute_path: string;
  contents: string;
  truncated: boolean;
  size_bytes: number;
}

export interface WorkspaceTextPreview {
  path: string;
  absolutePath: string;
  contents: string;
  truncated: boolean;
  sizeBytes: number;
}

function assertTauri() {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
}

export async function openWorkspacePath(path: string): Promise<void> {
  assertTauri();
  await invoke<void>('open_workspace_path', { path });
}

export async function revealWorkspacePath(path: string): Promise<void> {
  assertTauri();
  await invoke<void>('reveal_workspace_path', { path });
}

export async function previewWorkspaceText(path: string): Promise<WorkspaceTextPreview> {
  assertTauri();
  const preview = await invoke<RawWorkspaceTextPreview>('preview_workspace_text', { path });
  return {
    path: preview.path,
    absolutePath: preview.absolute_path,
    contents: preview.contents,
    truncated: preview.truncated,
    sizeBytes: preview.size_bytes,
  };
}

/**
 * Resolve a workspace-relative path to its canonical absolute path on disk,
 * after the Rust side validates it stays inside the active Neppy
 * workspace. Useful for UI flows that need to compose an absolute path into a
 * platform-specific URL scheme (e.g. `obsidian://open?path=<abs>`) without
 * re-implementing path normalization in the renderer.
 */
export async function resolveWorkspaceAbsolutePath(path: string): Promise<string> {
  assertTauri();
  return invoke<string>('resolve_workspace_absolute_path', { path });
}

/**
 * Open the OS-native folder picker; resolves to the chosen absolute path, or
 * `null` when the user cancelled.
 *
 * The renderer has no other way to obtain a directory path. `<input
 * type="file" webkitdirectory>` only ever yields file names plus a relative
 * path under the chosen root, and the `File.path` extension that Chromium adds
 * does not exist in Wry's WebKit (#5456) — so the host has to hand it over.
 */
export async function pickFolderViaDialog(): Promise<string | null> {
  assertTauri();
  return invoke<string | null>('pick_folder_via_dialog');
}

/**
 * Where the folder picker should start, or `null` when nothing plausible was
 * found. Resolved by the core, which owns the same root list the config
 * migration uses to repair sources that only stored a folder name.
 */
export async function defaultMemoryFolderPath(): Promise<string | null> {
  assertTauri();
  return invoke<string | null>('default_memory_folder_path');
}
