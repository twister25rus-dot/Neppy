import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  addPetGoal,
  buildPetDigestNow,
  decidePetProposal,
  dismissPetNote,
  fetchPetFeed,
  fetchPetInbox,
  fetchPetNotes,
  getPet,
  removePetGoal,
  runPetNow,
  updatePet,
} from './petApi';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

describe('petApi', () => {
  beforeEach(() => mockCallCoreRpc.mockReset());

  it('getPet calls pet_get and returns the bare value', async () => {
    mockCallCoreRpc.mockResolvedValue({ id: 'p1', name: 'Pet', enabled: false });
    const pet = await getPet();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.pet_get', params: {} });
    expect(pet.id).toBe('p1');
  });

  it('unwraps the { result, logs } envelope', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { id: 'p1', name: 'Pip' }, logs: ['x'] });
    const pet = await getPet();
    expect(pet.name).toBe('Pip');
  });

  it('updatePet sends the patch under `patch`', async () => {
    mockCallCoreRpc.mockResolvedValue({ id: 'p1' });
    await updatePet({ enabled: true, name: 'Pip' });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.pet_update',
      params: { patch: { enabled: true, name: 'Pip' } },
    });
  });

  it('goal add and remove use the contract param names', async () => {
    mockCallCoreRpc.mockResolvedValue({ id: 'g1' });
    await addPetGoal('Finish the portfolio');
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.pet_goal_add',
      params: { text: 'Finish the portfolio' },
    });
    mockCallCoreRpc.mockResolvedValue({ removed: true });
    await removePetGoal('g1');
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.pet_goal_remove',
      params: { goal_id: 'g1' },
    });
  });

  it('runPetNow defaults to wait=false', async () => {
    mockCallCoreRpc.mockResolvedValue({ status: 'started' });
    await runPetNow();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.pet_run_now',
      params: { wait: false },
    });
  });

  it('fetchPetFeed passes paging options and normalises missing fields', async () => {
    mockCallCoreRpc.mockResolvedValue({});
    const feed = await fetchPetFeed({ limit: 10 });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.pet_feed',
      params: { limit: 10 },
    });
    expect(feed).toEqual({ digests: [], notes: [], last_run: null });
  });

  it('fetchPetNotes passes the state filter and tolerates a non-array result', async () => {
    mockCallCoreRpc.mockResolvedValue(null);
    expect(await fetchPetNotes({ state: 'queued' })).toEqual([]);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.pet_notes_list',
      params: { state: 'queued' },
    });
  });

  it('dismissPetNote sends note_id', async () => {
    mockCallCoreRpc.mockResolvedValue({ id: 'n1', state: 'dismissed' });
    await dismissPetNote('n1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.pet_note_dismiss',
      params: { note_id: 'n1' },
    });
  });

  it('fetchPetInbox normalises missing arrays', async () => {
    mockCallCoreRpc.mockResolvedValue({ proposals: [{ id: 'p' }] });
    const inbox = await fetchPetInbox();
    expect(inbox.proposals).toHaveLength(1);
    expect(inbox.approvals).toEqual([]);
  });

  it('decidePetProposal sends proposal_id and decision', async () => {
    mockCallCoreRpc.mockResolvedValue({ proposal: { id: 'p1' }, chat_prompt: 'hello' });
    const out = await decidePetProposal('p1', 'accept');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.pet_proposal_decide',
      params: { proposal_id: 'p1', decision: 'accept' },
    });
    expect(out.chat_prompt).toBe('hello');
  });

  it('buildPetDigestNow may resolve to null', async () => {
    mockCallCoreRpc.mockResolvedValue(null);
    expect(await buildPetDigestNow()).toBeNull();
  });
});
