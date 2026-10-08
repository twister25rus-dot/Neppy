import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ReleasePreflight, ReleaseRecord } from '../../services/api/debugModeApi';
import { renderWithProviders } from '../../test/test-utils';
import { formatElapsed, ReleaseCard } from './ReleaseCard';
import { RELEASE_POLL_MS, validateReleaseVersion } from './useRelease';

const api = vi.hoisted(() => ({
  getReleasePreflight: vi.fn(),
  startRelease: vi.fn(),
  getReleaseStatus: vi.fn(),
}));
vi.mock('../../services/api/debugModeApi', () => api);
vi.mock('../../services/analytics', () => ({ trackEvent: vi.fn() }));
const openUrl = vi.hoisted(() => vi.fn());
vi.mock('../../utils/openUrl', () => ({ openUrl }));

const preflight = (over: Partial<ReleasePreflight> = {}): ReleasePreflight => ({
  project_root: '/repo',
  branch: 'main',
  release_branch: 'main',
  clean: true,
  behind: false,
  ahead_commits: 2,
  current_version: '0.68.5',
  suggested_version: '0.68.6',
  signing_key_present: true,
  gh_ready: true,
  fetch_error: null,
  blockers: [],
  last_tag: 'v0.68.5',
  ...over,
});

const record = (over: Partial<ReleaseRecord> = {}): ReleaseRecord => ({
  phase: 'idle',
  version: null,
  started_at: null,
  finished_at: null,
  exit_code: null,
  log_tail: '',
  tag: null,
  release_url: null,
  error: '',
  ...over,
});

const running = (over: Partial<ReleaseRecord> = {}) =>
  record({ phase: 'running', version: '0.68.6', started_at: new Date().toISOString(), ...over });

async function tick(ms = RELEASE_POLL_MS) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

describe('validateReleaseVersion', () => {
  it('requires X.Y.Z and a strictly greater numeric version', () => {
    expect(validateReleaseVersion('0.68.6', '0.68.5')).toBeNull();
    expect(validateReleaseVersion('0.68.10', '0.68.9')).toBeNull();
    expect(validateReleaseVersion('1.0.0', '0.99.99')).toBeNull();
    expect(validateReleaseVersion('0.68.5', '0.68.5')).toBe('notGreater');
    expect(validateReleaseVersion('0.68.4', '0.68.5')).toBe('notGreater');
    expect(validateReleaseVersion('0.9.0', '0.68.5')).toBe('notGreater');
    expect(validateReleaseVersion('1.2', '0.68.5')).toBe('format');
    expect(validateReleaseVersion('v1.2.3', '0.68.5')).toBe('format');
    expect(validateReleaseVersion('', '0.68.5')).toBe('format');
  });

  it('formats elapsed time', () => {
    expect(formatElapsed(0)).toBe('0:00');
    expect(formatElapsed(75)).toBe('1:15');
    expect(formatElapsed(3725)).toBe('1:02:05');
  });
});

describe('ReleaseCard', () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    Object.values(api).forEach(f => f.mockReset());
    openUrl.mockReset();
    openUrl.mockResolvedValue(undefined);
    api.getReleasePreflight.mockResolvedValue(preflight());
    api.getReleaseStatus.mockResolvedValue(record());
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows the current version and last tag, and enables publish when nothing blocks', async () => {
    renderWithProviders(<ReleaseCard />);
    expect(await screen.findByTestId('debug-release-current')).toHaveTextContent(
      'Current version 0.68.5'
    );
    expect(screen.getByTestId('debug-release-last-tag')).toHaveTextContent('Last release v0.68.5');
    expect(screen.queryByTestId('debug-release-blockers')).toBeNull();
    expect(screen.getByTestId('debug-release-publish')).toBeEnabled();
  });

  it('renders every blocker as a sentence and disables publish', async () => {
    api.getReleasePreflight.mockResolvedValue(
      preflight({
        release_branch: 'main',
        blockers: [
          'not_release_branch',
          'dirty',
          'behind',
          'no_signing_key',
          'gh_not_ready',
          'release_running',
          'nothing_to_release',
        ],
      })
    );
    renderWithProviders(<ReleaseCard />);

    const list = await screen.findByTestId('debug-release-blockers');
    expect(list).toHaveTextContent('Switch to the main branch first.');
    expect(list).toHaveTextContent('Commit or stash your uncommitted changes first.');
    expect(list).toHaveTextContent('Your branch is behind.');
    expect(list).toHaveTextContent('~/.neppy-updater/neppy.key');
    expect(list).toHaveTextContent('gh auth login');
    expect(list).toHaveTextContent('A release is already running.');
    expect(list).toHaveTextContent('There is nothing new to publish.');
    expect(list.querySelectorAll('li')).toHaveLength(7);
    expect(screen.getByTestId('debug-release-publish')).toBeDisabled();
    fireEvent.click(screen.getByTestId('debug-release-publish'));
    expect(screen.queryByTestId('debug-release-version')).toBeNull();
  });

  it('validates the version in the confirm dialog and starts only after confirm', async () => {
    api.startRelease.mockResolvedValue(running());
    renderWithProviders(<ReleaseCard />);

    fireEvent.click(await screen.findByTestId('debug-release-publish'));
    const input = (await screen.findByTestId('debug-release-version')) as HTMLInputElement;
    const confirm = screen.getByTestId('debug-release-confirm');

    // Prefilled with the suggestion and nothing has been started yet.
    expect(input.value).toBe('0.68.6');
    expect(confirm).toHaveTextContent('Publish v0.68.6');
    expect(confirm).toBeEnabled();
    expect(api.startRelease).not.toHaveBeenCalled();

    fireEvent.change(input, { target: { value: '0.68.4' } });
    expect(screen.getByTestId('debug-release-version-error')).toHaveTextContent(
      'The version must be higher than 0.68.5.'
    );
    expect(confirm).toBeDisabled();

    fireEvent.change(input, { target: { value: '1.2' } });
    expect(screen.getByTestId('debug-release-version-error')).toHaveTextContent(
      'Use the format X.Y.Z'
    );
    expect(confirm).toBeDisabled();
    fireEvent.click(confirm);
    expect(api.startRelease).not.toHaveBeenCalled();

    fireEvent.change(input, { target: { value: '0.68.7' } });
    expect(screen.queryByTestId('debug-release-version-error')).toBeNull();
    expect(confirm).toBeEnabled();
    expect(confirm).toHaveTextContent('Publish v0.68.7');
    fireEvent.click(confirm);

    await waitFor(() => expect(api.startRelease).toHaveBeenCalledTimes(1));
    expect(api.startRelease).toHaveBeenCalledWith('0.68.7');
    expect(await screen.findByTestId('debug-release-running')).toHaveTextContent(
      'Publishing v0.68.6'
    );
  });

  it('cancelling the dialog starts nothing', async () => {
    renderWithProviders(<ReleaseCard />);
    fireEvent.click(await screen.findByTestId('debug-release-publish'));
    fireEvent.click(await screen.findByTestId('debug-release-cancel'));
    await waitFor(() => expect(screen.queryByTestId('debug-release-version')).toBeNull());
    expect(api.startRelease).not.toHaveBeenCalled();
  });

  it('polls status only while running and stops once the release succeeded', async () => {
    api.startRelease.mockResolvedValue(running());
    renderWithProviders(<ReleaseCard />);
    await screen.findByTestId('debug-release-publish');

    // Idle: no polling, however long we wait.
    const idleCalls = api.getReleaseStatus.mock.calls.length;
    await tick(RELEASE_POLL_MS * 3);
    expect(api.getReleaseStatus.mock.calls.length).toBe(idleCalls);

    fireEvent.click(screen.getByTestId('debug-release-publish'));
    fireEvent.click(await screen.findByTestId('debug-release-confirm'));
    expect(await screen.findByTestId('debug-release-running')).toBeInTheDocument();

    // Running: one status read per interval, the log tail follows along.
    const preflightsBefore = api.getReleasePreflight.mock.calls.length;
    api.getReleaseStatus.mockResolvedValue(running({ log_tail: 'Building neppy_core' }));
    await tick();
    expect(api.getReleaseStatus.mock.calls.length).toBe(idleCalls + 1);
    expect(await screen.findByTestId('debug-release-log')).toHaveTextContent('Building neppy_core');
    expect(screen.getByTestId('debug-release-elapsed')).toBeInTheDocument();

    api.getReleaseStatus.mockResolvedValue(
      record({
        phase: 'succeeded',
        version: '0.68.6',
        exit_code: 0,
        tag: 'v0.68.6',
        release_url: 'https://github.com/example/neppy/releases/tag/v0.68.6',
      })
    );
    await tick();
    expect(await screen.findByTestId('debug-release-succeeded')).toHaveTextContent(
      'v0.68.6 published'
    );
    // The finished run changes what is releasable, so readiness is re-read.
    await waitFor(() =>
      expect(api.getReleasePreflight.mock.calls.length).toBeGreaterThan(preflightsBefore)
    );

    // Polling stopped with the run.
    const doneCalls = api.getReleaseStatus.mock.calls.length;
    await tick(RELEASE_POLL_MS * 3);
    expect(api.getReleaseStatus.mock.calls.length).toBe(doneCalls);
  });

  it('shows the tag and opens the release page externally on success', async () => {
    const url = 'https://github.com/example/neppy/releases/tag/v0.68.6';
    api.getReleaseStatus.mockResolvedValue(
      record({
        phase: 'succeeded',
        version: '0.68.6',
        exit_code: 0,
        tag: 'v0.68.6',
        release_url: url,
      })
    );
    renderWithProviders(<ReleaseCard />);

    expect(await screen.findByTestId('debug-release-succeeded')).toHaveTextContent(
      'v0.68.6 published'
    );
    expect(screen.getByTestId('debug-release')).toHaveTextContent('v0.68.6');
    const link = screen.getByTestId('debug-release-link');
    expect(link).toHaveAttribute('href', url);
    fireEvent.click(link);
    expect(openUrl).toHaveBeenCalledWith(url);

    fireEvent.click(screen.getByTestId('debug-release-dismiss'));
    expect(await screen.findByTestId('debug-release-publish')).toBeInTheDocument();
  });

  it('shows the error and log tail on failure, and "Try again" re-runs preflight', async () => {
    api.getReleaseStatus.mockResolvedValue(
      record({
        phase: 'failed',
        version: '0.68.6',
        exit_code: 1,
        error: 'gh release create failed',
        log_tail: 'step 3 of 5\nboom',
      })
    );
    renderWithProviders(<ReleaseCard />);

    expect(await screen.findByTestId('debug-release-failed')).toHaveTextContent(
      'The release failed.'
    );
    expect(screen.getByTestId('debug-release-failed-reason')).toHaveTextContent(
      'gh release create failed'
    );
    expect(screen.getByTestId('debug-release-log')).toHaveTextContent('boom');
    expect(screen.queryByTestId('debug-release-publish')).toBeNull();

    const before = api.getReleasePreflight.mock.calls.length;
    fireEvent.click(screen.getByTestId('debug-release-retry'));
    await waitFor(() => expect(api.getReleasePreflight.mock.calls.length).toBe(before + 1));
    expect(await screen.findByTestId('debug-release-publish')).toBeEnabled();
    expect(screen.queryByTestId('debug-release-failed')).toBeNull();
  });

  it('surfaces a start failure without throwing and stays idle', async () => {
    api.startRelease.mockRejectedValue(new Error('release already running'));
    renderWithProviders(<ReleaseCard />);

    fireEvent.click(await screen.findByTestId('debug-release-publish'));
    fireEvent.click(await screen.findByTestId('debug-release-confirm'));

    expect(await screen.findByTestId('debug-release-error')).toHaveTextContent(
      'Could not start the release: release already running'
    );
    expect(screen.getByTestId('debug-release-publish')).toBeEnabled();
  });

  it('surfaces a preflight failure as an error', async () => {
    api.getReleasePreflight.mockRejectedValue(new Error('no repo'));
    renderWithProviders(<ReleaseCard />);
    expect(await screen.findByTestId('debug-release-error')).toHaveTextContent(
      'Could not check release readiness: no repo'
    );
    expect(screen.queryByTestId('debug-release-publish')).toBeNull();
  });
});
