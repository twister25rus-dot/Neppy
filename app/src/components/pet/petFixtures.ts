import type {
  PetDigest,
  PetFeed,
  PetInbox,
  PetNote,
  PetProfile,
  PetProposal,
} from '../../services/api/petApi';

/** Test-only builders for Pet mode wire objects. */
export const makePet = (over: Partial<PetProfile> = {}): PetProfile => ({
  id: 'pet-1',
  name: 'Pip',
  persona: '',
  enabled: true,
  research_preset: 'standard',
  digest_time: '07:00',
  quiet_start: '22:00',
  quiet_end: '07:00',
  notify_budget_per_day: 3,
  sources: ['memory', 'tasks', 'composio', 'web'],
  goals: [],
  research_job_id: 'job-1',
  next_research_at: '2099-01-01T11:00:00Z',
  next_digest_at: '2099-01-02T06:00:00Z',
  last_pass_at: '2020-01-01T09:00:00Z',
  created_at: '2020-01-01T00:00:00Z',
  updated_at: '2020-01-01T00:00:00Z',
  ...over,
});

export const makeNote = (over: Partial<PetNote> = {}): PetNote => ({
  id: 'note-1',
  pet_id: 'pet-1',
  source: 'tasks',
  kind: 'deadline',
  title: 'Lesson plan due',
  body: 'Plan for Friday.',
  urgency: 2,
  due_at: '2099-01-03T09:00:00Z',
  goal_ids: [],
  proposed_action: null,
  fingerprint: 'abc123',
  injection_flagged: false,
  score: 53,
  bucket: 'digest',
  state: 'queued',
  digest_id: null,
  created_at: '2020-01-01T09:02:00Z',
  surfaced_at: null,
  notified_at: null,
  ...over,
});

export const makeDigest = (over: Partial<PetDigest> = {}): PetDigest => ({
  id: 'digest-1',
  pet_id: 'pet-1',
  created_at: '2020-01-01T09:03:00Z',
  local_date: '2020-01-01',
  body_md: '**Pip: your digest**\n\n- Lesson plan due',
  item_count: 2,
  withheld_count: 0,
  ...over,
});

export const makeProposal = (over: Partial<PetProposal> = {}): PetProposal => ({
  id: 'prop-1',
  pet_id: 'pet-1',
  note_id: 'note-1',
  note_title: 'Mentor asks for the draft',
  action_text: 'Reply to mentor with the draft date',
  state: 'pending',
  created_at: '2020-01-01T09:02:00Z',
  decided_at: null,
  expires_at: '2099-01-08T09:02:00Z',
  ...over,
});

export const makeFeed = (over: Partial<PetFeed> = {}): PetFeed => ({
  digests: [],
  notes: [],
  last_run: null,
  ...over,
});

export const makeInbox = (over: Partial<PetInbox> = {}): PetInbox => ({
  proposals: [],
  approvals: [],
  ...over,
});
