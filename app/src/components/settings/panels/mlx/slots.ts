/**
 * Model slots on an MLX server, and how a checkpoint is matched to one.
 *
 * `mlx_vlm.server` serves six slots from a single process, which is why models
 * are ticked rather than picked one at a time: a server can hold a chat model,
 * an embedder and a speech model simultaneously.
 */

/** Config field names on an `[[mlx.server]]` block, in display order. */
export const SLOTS = [
  'model',
  'embedding_model',
  'stt_model',
  'tts_model',
  'reranker_model',
  'image_model',
] as const;

export type Slot = (typeof SLOTS)[number];

/** i18n key for a slot's label. */
export const SLOT_LABEL_KEY: Record<Slot, string> = {
  model: 'mlx.slot.model',
  embedding_model: 'mlx.slot.embedding',
  stt_model: 'mlx.slot.stt',
  tts_model: 'mlx.slot.tts',
  reranker_model: 'mlx.slot.reranker',
  image_model: 'mlx.slot.image',
};

/**
 * Best guess at which slot a checkpoint belongs in, from its repo id.
 *
 * Advisory only — the guess is shown next to the tick and can be changed.
 * Naming is community convention rather than a standard, so a wrong guess is
 * expected for unusual repos and must stay correctable.
 *
 * Order matters: the more specific families are tested before the generic
 * chat fallback, and `reranker` before `embedding` because a reranker id
 * frequently contains both words.
 */
export function inferSlot(modelId: string): Slot {
  const id = modelId.toLowerCase();

  if (/whisper|parakeet|distil-whisper|wav2vec|speech.?to.?text|\bstt\b/.test(id)) {
    return 'stt_model';
  }
  if (/kokoro|piper|bark|xtts|outetts|speecht5|text.?to.?speech|\btts\b/.test(id)) {
    return 'tts_model';
  }
  if (/rerank/.test(id)) {
    return 'reranker_model';
  }
  if (/embed|\bbge\b|gte-|e5-|nomic|minilm/.test(id)) {
    return 'embedding_model';
  }
  if (/stable-diffusion|\bsdxl\b|flux|playground-v|kandinsky/.test(id)) {
    return 'image_model';
  }
  return 'model';
}

/** The slot a model currently occupies on a server, or `null` if unticked. */
export function occupiedSlot(
  modelId: string,
  slots: Partial<Record<Slot, string | null>>
): Slot | null {
  return SLOTS.find(slot => (slots[slot] ?? '') === modelId) ?? null;
}
