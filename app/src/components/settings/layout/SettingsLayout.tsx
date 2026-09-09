import debug from 'debug';
import { Outlet } from 'react-router-dom';

import { SidebarContent } from '../../layout/shell/SidebarSlot';
import { SettingsLayoutProvider } from './SettingsLayoutContext';
import SettingsSidebar from './SettingsSidebar';

const log = debug('settings:layout');

interface SettingsLayoutProps {
  /**
   * Keep the settings navigation inside this layout instead of projecting it
   * into the root app sidebar. Desktop's dialog presentation uses this so the
   * window remains a self-contained two-pane surface. Mobile and the routed
   * page presentation retain the existing sidebar-slot behavior.
   */
  inlineSidebar?: boolean;
}

/**
 * Settings shell, used by every target. In the normal routed presentation the
 * grouped navigation lives in the root app sidebar's dynamic region (projected
 * via {@link SidebarContent}). Desktop's dialog presentation keeps the same
 * navigation inline, beside the routed panel, so the pop-up is a complete
 * settings window rather than a floating content card with navigation behind
 * it.
 */
const SettingsLayout = ({ inlineSidebar = false }: SettingsLayoutProps) => {
  log('render');

  const panel = (
    <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-hidden">
        <Outlet />
      </div>
    </div>
  );

  if (inlineSidebar) {
    return (
      <SettingsLayoutProvider value={{ inTwoPaneShell: true }}>
        <div className="flex h-full min-h-0 w-full overflow-hidden">
          <aside
            className="h-full w-64 shrink-0 overflow-hidden border-r border-line-subtle bg-surface-chrome"
            data-testid="settings-dialog-sidebar">
            <SettingsSidebar />
          </aside>
          {panel}
        </div>
      </SettingsLayoutProvider>
    );
  }

  return (
    <SettingsLayoutProvider value={{ inTwoPaneShell: true }}>
      <SidebarContent>
        <div className="h-full overflow-hidden">
          <SettingsSidebar />
        </div>
      </SidebarContent>
      {/* Bounded flex column: the routed panel owns the only vertical scroll
          and renders its own header (title, description, sibling sub-nav).

          It renders flush, NOT inside a card. The panel used to sit on its own
          `rounded-2xl border bg-surface shadow-soft` sheet, which put a second
          bordered container inside the shell's content surface — a card on a
          card, with the page's own header and full-bleed divider inside the
          inner one. The shell already provides the surface; every other routed
          page uses it directly, and settings does now too. */}
      {/* No `max-w-*`/`mx-auto` here: capping the column and centring it reads
          as a left/right gutter the other routed pages do not have, which is
          what made settings look inset. The pane is the width of the content
          surface, same as Connections or Workflows. */}
      {panel}
    </SettingsLayoutProvider>
  );
};

export default SettingsLayout;
