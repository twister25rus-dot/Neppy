import { describe, expect, it } from 'vitest';

import chatRuntimeReducer, { setComposerModel } from './chatRuntimeSlice';

/**
 * The composer's model pick used to be component state, so it was gone on the
 * next launch and the model had to be picked again every time. It lives in the
 * store now and is whitelisted for persistence (see `chatRuntimePersistConfig`).
 */
describe('chatRuntime composer model', () => {
  const initial = chatRuntimeReducer(undefined, { type: '@@INIT' });

  it('starts with no pick, leaving usage-reported context authoritative', () => {
    expect(initial.composerModel).toBeNull();
    expect(initial.composerModelContextWindow).toBeUndefined();
  });

  it('records the picked model and its context window', () => {
    const next = chatRuntimeReducer(
      initial,
      setComposerModel({ model: 'mlx:ornith-ai/Ornith-1.5-9B-MLX-8bit', contextWindow: 32768 })
    );

    expect(next.composerModel).toBe('mlx:ornith-ai/Ornith-1.5-9B-MLX-8bit');
    expect(next.composerModelContextWindow).toBe(32768);
  });

  it('keeps the pick across unrelated actions', () => {
    const picked = chatRuntimeReducer(
      initial,
      setComposerModel({ model: 'ollama:qwen3:8b', contextWindow: null })
    );
    const later = chatRuntimeReducer(picked, { type: 'some/other/action' });

    expect(later.composerModel).toBe('ollama:qwen3:8b');
  });

  it('treats a model with no reported window as an unknown limit, not as no pick', () => {
    const next = chatRuntimeReducer(initial, setComposerModel({ model: 'mlx:some-model' }));

    expect(next.composerModel).toBe('mlx:some-model');
    expect(next.composerModelContextWindow).toBeNull();
  });

  it('clears back to the product routing when the pick is cleared', () => {
    const picked = chatRuntimeReducer(
      initial,
      setComposerModel({ model: 'mlx:some-model', contextWindow: 8192 })
    );
    const cleared = chatRuntimeReducer(picked, setComposerModel({ model: null }));

    expect(cleared.composerModel).toBeNull();
  });
});
