import { isMac } from '../../../lib/commands/shortcut';
import { isTauri } from '../../../utils/tauriCommands/common';

/**
 * Height (px) of the drag strip.
 *
 * 28px is the traffic-light zone itself, which put the first control flush
 * against the bottom of the lights — technically clear, visually crowded, and
 * it read as the window overlapping its own chrome. 40px clears the band and
 * leaves the gap the lights need to look deliberate rather than avoided.
 */
export const WINDOW_DRAG_BAR_HEIGHT = 40;

/**
 * Whether macOS is painting its window controls over our content.
 *
 * The two things that make it true are independent of each other — the overlay
 * title bar is a macOS window style, and outside Tauri there is no window at
 * all — so both are checked in one place rather than at each call site.
 */
export function hasOverlayWindowControls(): boolean {
  return isTauri() && isMac();
}

/**
 * A draggable strip the height of the window-control band, or nothing at all
 * where the platform draws its own title bar.
 *
 * Chrome at the top of the sidebar has to sit below this, not beside it: the
 * traffic lights are ~70px wide and the column can be dragged down to 188px, so
 * "right-align the icons and they will stay clear" holds at the default width and
 * breaks at the narrow end — and even when it holds, the icons still share the
 * band with the lights, which is what reads as UI sitting on the window
 * controls. Reserving the band costs 28px of column height and cannot collide at
 * any width, in either sidebar state.
 */
export function WindowControlsSpacer({ className }: { className?: string }) {
  if (!hasOverlayWindowControls()) return null;
  return (
    <div
      data-tauri-drag-region
      data-testid="window-controls-spacer"
      aria-hidden="true"
      className={`w-full flex-none ${className ?? ''}`}
      style={{ height: WINDOW_DRAG_BAR_HEIGHT }}
    />
  );
}

/**
 * Transparent macOS window-drag band for the overlay title bar.
 *
 * The main window runs with `titleBarStyle: "Overlay"` + `hiddenTitle` (see
 * `app/src-tauri/tauri.conf.json`), so macOS draws transparent traffic lights
 * over the web content but does NOT make the top draggable on its own — the
 * webview captures the pointer events. We opt back in with a `data-tauri-drag-
 * region` band.
 *
 * Positioned over the top of the content column ({@link RootShellLayout}), so
 * it does not reserve vertical space or add an inherited top inset to routed
 * pages. The window controls overlay this same title-bar region.
 *
 * Native CEF provider webviews composite above all HTML and so can't be dragged
 * through; that's a platform limit, not this band. The sidebar is intentionally
 * excluded — its header already drags in place.
 *
 * macOS-only: Windows/Linux keep their native decorated title bar (the
 * `Overlay` style is a no-op there), so reserving a band would only waste
 * vertical space. Outside the Tauri runtime (browser/iOS) there is no window to
 * drag, so it renders nothing.
 */
export default function WindowDragBar() {
  if (!hasOverlayWindowControls()) return null;
  return (
    <div
      data-tauri-drag-region
      aria-hidden="true"
      className="absolute inset-x-0 top-0 z-10"
      style={{ height: WINDOW_DRAG_BAR_HEIGHT }}
    />
  );
}
