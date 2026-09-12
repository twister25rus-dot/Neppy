import { describe, expect, it } from 'vitest';

import reducer, { setVisibleModels, toggleVisibleModel } from '../chatRuntimeSlice';

const initial = () => reducer(undefined, { type: '@@INIT' });

describe('chatRuntimeSlice — visibleModels', () => {
  it('starts empty, which the picker reads as "nothing pinned yet"', () => {
    expect(initial().visibleModels).toEqual([]);
  });

  it('toggles a key on and back off', () => {
    const on = reducer(initial(), toggleVisibleModel('openai:gpt-5'));
    expect(on.visibleModels).toEqual(['openai:gpt-5']);

    const off = reducer(on, toggleVisibleModel('openai:gpt-5'));
    expect(off.visibleModels).toEqual([]);
  });

  it('keeps the order models were pinned in', () => {
    // The picker renders this list as-is, so the order is the user's, not a
    // sort — pinning a second model must not reshuffle the first.
    let state = reducer(initial(), toggleVisibleModel('anthropic:claude-opus-5'));
    state = reducer(state, toggleVisibleModel('openai:gpt-5'));
    expect(state.visibleModels).toEqual(['anthropic:claude-opus-5', 'openai:gpt-5']);
  });

  it('replaces the whole set for a bulk change', () => {
    const state = reducer(initial(), setVisibleModels(['ollama:llama3', 'openai:gpt-5']));
    expect(state.visibleModels).toEqual(['ollama:llama3', 'openai:gpt-5']);
  });
});
