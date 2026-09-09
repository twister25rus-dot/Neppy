import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import SettingsTabbedPage from '../layout/SettingsTabbedPage';
import AIPanel, { type AIPanelTab } from './AIPanel';

/**
 * The Connections → LLM surface: provider credentials and workload routing.
 *
 * Provider credentials and workload routing share one continuous setup tab.
 * MLX remains separate because it is a runtime-specific surface. Retired
 * developer diagnostics are intentionally not recreated here.
 */
const LlmConnectionsPanel = () => {
  const { t } = useT();
  const [tab, setTab] = useState<AIPanelTab>('providers');

  return (
    <SettingsTabbedPage
      title={t('pages.settings.ai.llm')}
      description={t('connections.header.llm')}
      tabs={[
        { id: 'providers', label: t('connections.llm.apiKeys') },
        { id: 'mlx', label: t('settings.ai.mlx') },
      ]}
      value={tab}
      onChange={setTab}
      tabsAriaLabel={t('pages.settings.ai.llm')}
      tabsTestIdPrefix="ai-tab">
      <AIPanel tab={tab} onTabChange={setTab} hideTabChrome combineProvidersAndRouting />
    </SettingsTabbedPage>
  );
};

export default LlmConnectionsPanel;
