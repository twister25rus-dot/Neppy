import { useState } from 'react';

import { TabsContent, TabsList, TabsRoot, TabsTrigger } from '../../../components/ui';
import { useT } from '../../../lib/i18n/I18nContext';
import { CheckpointList } from './CheckpointList';
import { DiffViewer } from './DiffViewer';
import { TaskHistory } from './TaskHistory';

type PanelTab = 'diff' | 'history' | 'checkpoints';

/** Diff / History / Checkpoints tabs. Bump `refreshKey` to re-fetch the active panel. */
export function DebugPanels({ refreshKey = 0 }: { refreshKey?: number }) {
  const { t } = useT();
  const [tab, setTab] = useState<PanelTab>('diff');

  return (
    <TabsRoot value={tab} onValueChange={v => setTab(v as PanelTab)} data-testid="debug-panels">
      <TabsList variant="line" aria-label={t('debug.panels.tabsLabel')}>
        <TabsTrigger value="diff" data-analytics-id="debug-panels-tab-diff">
          {t('debug.panels.tab.diff')}
        </TabsTrigger>
        <TabsTrigger value="history" data-analytics-id="debug-panels-tab-history">
          {t('debug.panels.tab.history')}
        </TabsTrigger>
        <TabsTrigger value="checkpoints" data-analytics-id="debug-panels-tab-checkpoints">
          {t('debug.panels.tab.checkpoints')}
        </TabsTrigger>
      </TabsList>
      <TabsContent value="diff" className="pt-3">
        <DiffViewer refreshKey={refreshKey} />
      </TabsContent>
      <TabsContent value="history" className="pt-3">
        <TaskHistory refreshKey={refreshKey} />
      </TabsContent>
      <TabsContent value="checkpoints" className="pt-3">
        <CheckpointList refreshKey={refreshKey} />
      </TabsContent>
    </TabsRoot>
  );
}

export default DebugPanels;
