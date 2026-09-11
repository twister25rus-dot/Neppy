import { type RefObject, useEffect, useLayoutEffect, useRef } from 'react';

/** Long enough to read as a move rather than a flicker, short of feeling slow. */
const DEFAULT_DURATION_MS = 600;

/**
 * Symmetric ease (the material "standard" curve), not an ease-out.
 *
 * An ease-out spends most of its time finishing: it covers half the distance in
 * the first fifth of the duration, which is why the earlier pass still read as a
 * jump followed by a settle no matter how long the duration got. This one
 * accelerates and decelerates, so the travel itself is what you see.
 */
const EASE = 'cubic-bezier(0.4, 0, 0.2, 1)';

/**
 * Animate the composer's drop from the middle of a new session to the bottom.
 *
 * The two declarations that actually move it — `justify-content: center` on the
 * column and `margin-top: auto` on the footer — are both non-animatable (`auto`
 * has no interpolable value), so a `transition` naming them does nothing at all.
 * That is why the first attempt at this animation was invisible: the only thing
 * left running was an 8px fade on the message group, and the composer itself
 * still teleported in a single frame.
 *
 * So measure the move instead of declaring it (FLIP). The element's position is
 * recorded on every commit; on the commit that fills the session we know where
 * it used to be, put it back there with a transform, and then release it to its
 * real place over one transition. The distance is always exactly right because
 * it was measured, not guessed — which matters here, since it depends on the
 * window height and on how tall the composer has grown.
 *
 * Returns the ref to attach to the element that moves.
 */
export function useFirstTurnDrop<T extends HTMLElement>(
  isEmpty: boolean,
  durationMs: number = DEFAULT_DURATION_MS
): RefObject<T | null> {
  const ref = useRef<T | null>(null);
  /** Where the element sat at the previous commit — the "First" of FLIP. */
  const previousTop = useRef<number | null>(null);
  const wasEmpty = useRef(isEmpty);
  /** Backstop that clears the inline styles if `transitionend` never arrives. */
  const settleTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Deliberately unconditional (no dependency array): the position has to be
  // recorded after *every* commit, because the commit that matters is only
  // recognisable by comparing against the one before it. Runs before paint, so
  // the parked transform is never visible in its untransformed place.
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;

    const top = node.getBoundingClientRect().top;
    const justFilled = wasEmpty.current && !isEmpty;
    const from = previousTop.current;
    wasEmpty.current = isEmpty;
    previousTop.current = top;

    if (!justFilled || from === null) return;
    // Sub-pixel moves are layout noise, not the drop.
    if (Math.abs(from - top) < 2) return;
    if (window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches) return;

    node.style.transition = 'none';
    node.style.transform = `translateY(${from - top}px)`;
    // Read back to force the parked position into its own style change; without
    // it the browser coalesces both writes into one frame and nothing animates.
    void node.getBoundingClientRect().top;
    node.style.transition = `transform ${durationMs}ms ${EASE}`;
    node.style.transform = 'translateY(0px)';

    // Hand the element back its own styling when the travel is over. NOT via
    // this effect's cleanup: with no dependency array that cleanup runs on the
    // very next commit — which a streaming turn produces within milliseconds —
    // so a listener removed there would never fire and the inline transform
    // would be left on the node. `once` plus a timer that outlives the
    // transition covers both the normal end and a transition that never
    // finishes (a backgrounded tab drops the event).
    if (settleTimer.current) clearTimeout(settleTimer.current);
    const done = () => {
      if (settleTimer.current) {
        clearTimeout(settleTimer.current);
        settleTimer.current = null;
      }
      node.style.transition = '';
      node.style.transform = '';
    };
    node.addEventListener('transitionend', done, { once: true });
    settleTimer.current = setTimeout(done, durationMs + 100);
  });

  // The timer is the only thing here that can outlive the component.
  useEffect(
    () => () => {
      if (settleTimer.current) clearTimeout(settleTimer.current);
    },
    []
  );

  return ref;
}
