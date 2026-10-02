import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';
import type { PendingApproval } from './approvalApi';

// ---------------------------------------------------------------------------
// Pet mode RPC client (`openhuman.pet_*`).
//
// The wire shapes mirror the backend contract. Handlers return the bare JSON
// value, but the core may also hand back the CLI-compatible `{ result, logs }`
// envelope, so every call goes through `unwrapValue`.
//
// Privacy: only method names are logged here. Note titles, bodies, goal text
// and digest bodies never reach a log line.
// ---------------------------------------------------------------------------

const log = debug('openhuman:petApi');

export type ResearchPreset = 'light' | 'standard' | 'frequent';
/**
 * Sources the pet may read. `composio` is stored and shown, but the research
 * lane has no direct mail or calendar tools: that content reaches the pet only
 * through what has already been synced into memory.
 */
export type PetSource = 'memory' | 'tasks' | 'composio' | 'web';

export interface PetGoal {
  id: string;
  text: string;
  created_at: string;
}

export interface PetProfile {
  id: string;
  name: string;
  persona: string;
  enabled: boolean;
  research_preset: ResearchPreset;
  /** `HH:MM`, device-local. */
  digest_time: string;
  quiet_start: string;
  quiet_end: string;
  notify_budget_per_day: number;
  sources: PetSource[];
  goals: PetGoal[];
  research_job_id: string | null;
  next_research_at: string | null;
  next_digest_at: string | null;
  last_pass_at: string | null;
  created_at: string;
  updated_at: string;
}

export type PetProfilePatch = Partial<
  Pick<
    PetProfile,
    | 'name'
    | 'persona'
    | 'enabled'
    | 'research_preset'
    | 'digest_time'
    | 'quiet_start'
    | 'quiet_end'
    | 'notify_budget_per_day'
    | 'sources'
  >
>;

export type PetNoteKind =
  | 'deadline'
  | 'request'
  | 'meeting'
  | 'change'
  | 'fyi'
  | 'idea'
  | 'proposal';
export type PetNoteSource = 'memory' | 'tasks' | 'calendar' | 'email' | 'web' | 'desktop' | 'other';
export type PetNoteState = 'new' | 'notified' | 'queued' | 'digested' | 'dropped' | 'dismissed';

export interface PetNote {
  id: string;
  pet_id: string;
  source: PetNoteSource;
  kind: PetNoteKind;
  title: string;
  /** Untrusted text gathered from the user's data: render as plain text only. */
  body: string;
  urgency: 0 | 1 | 2 | 3;
  due_at: string | null;
  goal_ids: string[];
  proposed_action: string | null;
  fingerprint: string;
  injection_flagged: boolean;
  score: number | null;
  bucket: 'notify' | 'digest' | 'drop' | 'duplicate' | null;
  state: PetNoteState;
  digest_id: string | null;
  created_at: string;
  surfaced_at: string | null;
  notified_at: string | null;
}

export interface PetDigest {
  id: string;
  pet_id: string;
  created_at: string;
  local_date: string;
  body_md: string;
  item_count: number;
  withheld_count: number;
}

export type PetProposalState = 'pending' | 'accepted' | 'dismissed' | 'expired';

export interface PetProposal {
  id: string;
  pet_id: string;
  note_id: string;
  note_title: string;
  action_text: string;
  state: PetProposalState;
  created_at: string;
  decided_at: string | null;
  expires_at: string;
}

export interface PetRunSummary {
  run_id: string | null;
  status: 'started' | 'completed' | 'failed';
  trigger: 'scheduled' | 'manual';
  finished_at: string | null;
  notes_seen: number;
  notified: number;
  queued: number;
  dropped: number;
  digest_id: string | null;
}

export interface PetFeed {
  digests: PetDigest[];
  notes: PetNote[];
  last_run: PetRunSummary | null;
}

export interface PetInbox {
  proposals: PetProposal[];
  approvals: PendingApproval[];
}

export interface PetPageOpts {
  limit?: number;
  /** RFC3339 `created_at` cursor. */
  before?: string;
}

export interface PetNotesQuery extends PetPageOpts {
  state?: PetNoteState;
}

export interface PetProposalDecision {
  proposal: PetProposal;
  /** Seed for a new chat composer on accept. Never auto-send it. */
  chat_prompt: string | null;
}

const unwrapValue = <T>(raw: unknown): T => {
  if (raw && typeof raw === 'object' && !Array.isArray(raw) && 'result' in raw) {
    return (raw as { result: T }).result;
  }
  return raw as T;
};

async function call<T>(fn: string, params: Record<string, unknown> = {}): Promise<T> {
  log('rpc %s', fn);
  const raw = await callCoreRpc<unknown>({ method: `openhuman.pet_${fn}`, params });
  return unwrapValue<T>(raw);
}

/** Fetch the primary pet, lazily created (disabled) on first call. */
export const getPet = (): Promise<PetProfile> => call<PetProfile>('get');

export const updatePet = (patch: PetProfilePatch): Promise<PetProfile> =>
  call<PetProfile>('update', { patch });

export const addPetGoal = (text: string): Promise<PetGoal> => call<PetGoal>('goal_add', { text });

export const removePetGoal = (goalId: string): Promise<{ removed: boolean }> =>
  call<{ removed: boolean }>('goal_remove', { goal_id: goalId });

/** Start a pass now. `wait=false` returns immediately with `status: "started"`. */
export const runPetNow = (wait = false): Promise<PetRunSummary> =>
  call<PetRunSummary>('run_now', { wait });

export const fetchPetFeed = async (opts: PetPageOpts = {}): Promise<PetFeed> => {
  const feed = await call<PetFeed>('feed', { ...opts });
  return {
    digests: feed?.digests ?? [],
    notes: feed?.notes ?? [],
    last_run: feed?.last_run ?? null,
  };
};

export const fetchPetNotes = async (opts: PetNotesQuery = {}): Promise<PetNote[]> => {
  const notes = await call<PetNote[]>('notes_list', { ...opts });
  return Array.isArray(notes) ? notes : [];
};

export const dismissPetNote = (noteId: string): Promise<PetNote> =>
  call<PetNote>('note_dismiss', { note_id: noteId });

export const fetchPetInbox = async (): Promise<PetInbox> => {
  const inbox = await call<PetInbox>('inbox_list');
  return { proposals: inbox?.proposals ?? [], approvals: inbox?.approvals ?? [] };
};

export const decidePetProposal = (
  proposalId: string,
  decision: 'accept' | 'dismiss'
): Promise<PetProposalDecision> =>
  call<PetProposalDecision>('proposal_decide', { proposal_id: proposalId, decision });

/** Build a digest from queued notes now. `null` when there is nothing to digest. */
export const buildPetDigestNow = (): Promise<PetDigest | null> =>
  call<PetDigest | null>('digest_now');
