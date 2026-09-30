/**
 * PetPage: Pet mode. A read-only background helper that scans the user's
 * memory and tasks while the Mac is awake, leaves a ranked digest, and raises
 * suggestions that only ever become a chat draft for the user to review.
 *
 * Tabs live in the URL (`?tab=feed|inbox|notes|settings`, default `feed`), and
 * `note` / `digest` query params pre-select an item so pet notifications can
 * deep link straight to it.
 */
import debug from 'debug';
import { useCallback, useState } from 'react';
import { useSearchParams } from 'react-router-dom';

import PageSectionHeader from '../components/layout/PageSectionHeader';
import PanelPage from '../components/layout/PanelPage';
import PetFeedTab from '../components/pet/PetFeedTab';
import { errorText } from '../components/pet/petFormat';
import PetHeader from '../components/pet/PetHeader';
import PetInboxTab from '../components/pet/PetInboxTab';
import PetNotesTab from '../components/pet/PetNotesTab';
import PetSettingsTab from '../components/pet/PetSettingsTab';
import { usePet } from '../components/pet/usePet';
import Badge from '../components/ui/Badge';
import Button from '../components/ui/Button';
import { CenteredLoadingState, ErrorBanner } from '../components/ui/LoadingState';
import { TabsContent, TabsList, TabsRoot, TabsTrigger } from '../components/ui/Tabs';
import { useT } from '../lib/i18n/I18nContext';
import { runPetNow } from '../services/api/petApi';

const log = debug('pet:page');

const TABS = ['feed', 'inbox', 'notes', 'settings'] as const;
type PetTab = (typeof TABS)[number];

const isTab = (value: string | null): value is PetTab =>
  value !== null && (TABS as readonly string[]).includes(value);

export default function PetPage() {
  const { t } = useT();
  const [params, setParams] = useSearchParams();
  const { pet, feed, inbox, loading, loadError, tick, refresh, applyPet } = usePet();
  const [running, setRunning] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [runError, setRunError] = useState<string | null>(null);

  const rawTab = params.get('tab');
  const tab: PetTab = isTab(rawTab) ? rawTab : 'feed';
  const noteId = params.get('note');
  const digestId = params.get('digest');

  const changeTab = useCallback(
    (next: string) => {
      if (!isTab(next)) return;
      log('tab -> %s', next);
      setParams({ tab: next }, { replace: true });
    },
    [setParams]
  );

  const openNote = useCallback(
    (id: string | null) => {
      const next: Record<string, string> = { tab: 'notes' };
      if (id) next.note = id;
      setParams(next, { replace: true });
    },
    [setParams]
  );

  const handleRunNow = async () => {
    setRunning(true);
    setNotice(null);
    setRunError(null);
    log('run now: start');
    try {
      const summary = await runPetNow(false);
      log('run now: status=%s', summary.status);
      setNotice(t('pet.status.runStarted'));
      await refresh();
    } catch (err) {
      log('run now failed: %o', err);
      setRunError(
        errorText(err).toLowerCase().includes('already running')
          ? t('pet.errors.alreadyRunning')
          : t('pet.errors.runFailed')
      );
    } finally {
      setRunning(false);
    }
  };

  const pendingCount =
    (inbox?.proposals.filter(p => p.state === 'pending').length ?? 0) +
    (inbox?.approvals.length ?? 0);

  const refreshSilently = useCallback(() => void refresh(), [refresh]);

  let body;
  if (loading && !pet) {
    body = <CenteredLoadingState label={t('pet.loading')} />;
  } else if (!pet || !feed || !inbox) {
    body = (
      <div data-testid="pet-load-error">
        <ErrorBanner
          message={t('pet.errors.loadFailed')}
          action={
            <Button
              type="button"
              variant="secondary"
              size="xs"
              analyticsId="pet-retry"
              onClick={refreshSilently}>
              {t('pet.actions.retry')}
            </Button>
          }
        />
      </div>
    );
  } else {
    body = (
      <>
        {loadError && (
          <ErrorBanner
            message={t('pet.errors.loadFailed')}
            action={
              <Button
                type="button"
                variant="secondary"
                size="xs"
                analyticsId="pet-retry"
                onClick={refreshSilently}>
                {t('pet.actions.retry')}
              </Button>
            }
          />
        )}
        <PetHeader
          pet={pet}
          running={running}
          notice={notice}
          error={runError}
          onRunNow={() => void handleRunNow()}
        />
        <TabsRoot value={tab} onValueChange={changeTab} className="space-y-4">
          <TabsList variant="line" aria-label={t('pet.tabs.aria')}>
            <TabsTrigger value="feed" data-testid="pet-tab-feed">
              {t('pet.tabs.feed')}
            </TabsTrigger>
            <TabsTrigger value="inbox" data-testid="pet-tab-inbox">
              {t('pet.tabs.inbox')}
              {pendingCount > 0 && <Badge variant="primary">{pendingCount}</Badge>}
            </TabsTrigger>
            <TabsTrigger value="notes" data-testid="pet-tab-notes">
              {t('pet.tabs.notes')}
            </TabsTrigger>
            <TabsTrigger value="settings" data-testid="pet-tab-settings">
              {t('pet.tabs.settings')}
            </TabsTrigger>
          </TabsList>
          <TabsContent value="feed">
            <PetFeedTab
              pet={pet}
              feed={feed}
              highlightDigestId={digestId}
              highlightNoteId={noteId}
              onOpenNote={openNote}
              onViewAllNotes={() => openNote(null)}
              onChanged={refreshSilently}
            />
          </TabsContent>
          <TabsContent value="inbox">
            <PetInboxTab inbox={inbox} onChanged={refreshSilently} />
          </TabsContent>
          <TabsContent value="notes">
            <PetNotesTab
              selectedId={noteId}
              onSelect={openNote}
              refreshKey={tick}
              onChanged={refreshSilently}
            />
          </TabsContent>
          <TabsContent value="settings">
            <PetSettingsTab pet={pet} onSaved={applyPet} onChanged={refreshSilently} />
          </TabsContent>
        </TabsRoot>
      </>
    );
  }

  return (
    <PanelPage testId="pet-page" contentClassName="p-4">
      <div className="mx-auto w-full max-w-3xl space-y-4">
        <PageSectionHeader title={t('pet.title')} description={t('pet.subtitle')} />
        {body}
      </div>
    </PanelPage>
  );
}
