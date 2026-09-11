/**
 * The measured half of the new-session composer drop.
 *
 * jsdom reports every box as 0×0, so the test drives `getBoundingClientRect`
 * itself — which is exactly the seam the hook depends on, and the reason a
 * declared `transition` could never have worked here: the distance is whatever
 * the layout says it is, and nothing in CSS can name it.
 */
import { render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { useFirstTurnDrop } from './useFirstTurnDrop';

/**
 * Report every box's `top` from a mutable cell.
 *
 * Patched on the prototype rather than on the node: the hook measures during the
 * *first* layout effect, before any per-node stub a test component could attach,
 * and a first measurement of jsdom's 0 would make every later move look like a
 * 500px drop.
 */
function stubLayout(cell: { top: number }) {
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(
    () => ({ top: cell.top, height: 0 }) as DOMRect
  );
}

function Harness({ isEmpty }: { isEmpty: boolean }) {
  const ref = useFirstTurnDrop<HTMLDivElement>(isEmpty, 400);
  return <div ref={ref} data-testid="footer" />;
}

afterEach(() => vi.restoreAllMocks());

describe('useFirstTurnDrop', () => {
  it('parks the element where it was and releases it to where it now is', () => {
    const cell = { top: 300 };
    stubLayout(cell);
    const { getByTestId, rerender } = render(<Harness isEmpty />);
    const footer = getByTestId('footer');

    // Both writes land in one synchronous layout effect, so the parked offset is
    // only observable as the previous value of a style mutation. It is the
    // assertion that matters: an end state of `translateY(0px)` would also hold
    // if the element had been parked at the wrong distance, or at none.
    const observer = new MutationObserver(() => {});
    observer.observe(footer, {
      attributes: true,
      attributeFilter: ['style'],
      attributeOldValue: true,
    });

    // The first turn lands: the composer's real position moves to the bottom.
    cell.top = 700;
    rerender(<Harness isEmpty={false} />);

    const seen = observer.takeRecords().map(record => record.oldValue ?? '');
    observer.disconnect();
    // Parked exactly the 400px it is about to travel (300 → 700), then released.
    expect(seen.some(value => value.includes('translateY(-400px)'))).toBe(true);
    expect(footer.style.transform).toBe('translateY(0px)');
    expect(footer.style.transition).toContain('400ms');

    footer.dispatchEvent(new Event('transitionend'));
    expect(footer.style.transform).toBe('');
    expect(footer.style.transition).toBe('');
  });

  it('does nothing when the position did not actually change', () => {
    const cell = { top: 500 };
    stubLayout(cell);
    const { getByTestId, rerender } = render(<Harness isEmpty />);
    const footer = getByTestId('footer');

    rerender(<Harness isEmpty={false} />);

    // A state flip that moved nothing is not a drop; animating it would flash
    // the composer for no reason.
    expect(footer.style.transform).toBe('');
    expect(footer.style.transition).toBe('');
  });

  it('does nothing for a viewer who asked for reduced motion', () => {
    vi.spyOn(window, 'matchMedia').mockImplementation(
      query =>
        ({
          matches: query.includes('prefers-reduced-motion'),
          media: query,
          addEventListener: () => {},
          removeEventListener: () => {},
        }) as unknown as MediaQueryList
    );

    const cell = { top: 300 };
    stubLayout(cell);
    const { getByTestId, rerender } = render(<Harness isEmpty />);
    const footer = getByTestId('footer');

    cell.top = 700;
    rerender(<Harness isEmpty={false} />);

    expect(footer.style.transform).toBe('');
  });

  it('leaves the element alone while the session stays empty', () => {
    const cell = { top: 300 };
    stubLayout(cell);
    const { getByTestId, rerender } = render(<Harness isEmpty />);
    const footer = getByTestId('footer');

    // A composer growing as the user types moves the box without filling the
    // session; only the empty→filled commit is a drop.
    cell.top = 260;
    rerender(<Harness isEmpty />);

    expect(footer.style.transform).toBe('');
  });
});
