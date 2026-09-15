import { useState } from 'react';
import { useLocation } from 'react-router-dom';

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
  // `#mlx` opens straight on the MLX tab, so the quick MLX menu's "MLX
  // settings" link lands where it says it will rather than on Providers with
  // one more click to find. Read once, as the initial value: the tab is the
  // user's after that, and re-reading would drag them back on every render.
  const { hash } = useLocation();
  const [tab, setTab] = useState<AIPanelTab>(hash === '#mlx' ? 'mlx' : 'providers');

  return (
    <SettingsTabbedPage
      narrow
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
