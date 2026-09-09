import { LuX } from 'react-icons/lu';
import { Route, Routes } from 'react-router-dom';

import SettingsLayout from '../components/settings/layout/SettingsLayout';
import { settingsRouteElements } from '../components/settings/settingsRouteElements';
import Button from '../components/ui/Button';
import { DialogClose, DialogContent, DialogRoot, DialogTitle } from '../components/ui/Dialog';
import { useT } from '../lib/i18n/I18nContext';

/**
 * Settings keeps one shared route table while supporting two presentations:
 * mobile uses the existing full-page route, and desktop places the same routed
 * panels inside a large, self-contained dialog over the previous page.
 */
interface SettingsProps {
  presentation?: 'page' | 'dialog';
  onClose?: () => void;
}

const SettingsRoutes = ({ inlineSidebar = false }: { inlineSidebar?: boolean }) => (
  // h-full chains the host height down to SettingsLayout so its panes can
  // bound to the viewport and scroll internally.
  <div className="h-full">
    <Routes>
      <Route element={<SettingsLayout inlineSidebar={inlineSidebar} />}>
        {settingsRouteElements()}
      </Route>
    </Routes>
  </div>
);

const Settings = ({ presentation = 'page', onClose }: SettingsProps) => {
  const { t } = useT();

  if (presentation === 'dialog') {
    return (
      <DialogRoot open onOpenChange={open => !open && onClose?.()}>
        <DialogContent
          aria-describedby={undefined}
          overlayClassName="bg-surface-overlay/70 backdrop-blur-md"
          className="h-[calc(100vh-5.5rem)] w-[calc(100vw-5rem)] max-w-[112rem] overflow-visible bg-transparent shadow-none">
          <DialogTitle className="sr-only">{t('nav.settings')}</DialogTitle>
          <DialogClose asChild>
            <Button
              iconOnly
              variant="secondary"
              size="lg"
              aria-label={t('common.close')}
              analyticsId="settings-dialog-close"
              className="absolute -top-14 right-0 rounded-xl bg-surface-chrome shadow-soft">
              <LuX className="h-5 w-5" />
            </Button>
          </DialogClose>
          <div
            className="h-full overflow-hidden rounded-3xl border border-line bg-surface shadow-large"
            data-testid="settings-dialog-window">
            <SettingsRoutes inlineSidebar />
          </div>
        </DialogContent>
      </DialogRoot>
    );
  }

  return <SettingsRoutes />;
};

export default Settings;
