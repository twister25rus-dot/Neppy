import { useT } from '../../../lib/i18n/I18nContext';
import { Alert } from '../../ui';
import Button from '../../ui/Button';
import { SettingsBadge, SettingsRow, SettingsSection, SettingsStatusLine } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
import DebugModeExternalPaths from './DebugModeExternalPaths';
import { DebugModeRepairsField, DebugModeRootField } from './DebugModeFields';
import { useDebugModeSettings } from './DebugModeSettingsState';
import DebugModeToggleRow from './DebugModeToggleRow';

const ALWAYS_ON = ['readFiles', 'modifyFiles', 'runTests', 'runBuilds'] as const;

/**
 * Settings → Debug Mode. Permissions for the in-app development agent. Every
 * change saves immediately as a minimal patch; dangerous capabilities live in
 * their own coral-marked section and default off in the core.
 */
const DebugModePanel = () => {
  const { t } = useT();
  const { settings, loading, loadError, saveError, saving, savedAt, reload, update } =
    useDebugModeSettings(t('settings.debugMode.loadError'), t('settings.debugMode.saveError'));

  if (loading) {
    return (
      <SettingsPanel description={t('settings.debugMode.menuDesc')}>
        <p className="text-sm text-content-muted">{t('settings.debugMode.loading')}</p>
      </SettingsPanel>
    );
  }

  if (!settings) {
    return (
      <SettingsPanel description={t('settings.debugMode.menuDesc')}>
        <div className="space-y-3" data-testid="debug-settings-load-error">
          <p className="text-sm text-coral-600 dark:text-coral-300">{loadError}</p>
          <Button
            variant="secondary"
            size="sm"
            analyticsId="settings-debug-mode-retry"
            onClick={reload}>
            {t('settings.debugMode.retry')}
          </Button>
        </div>
      </SettingsPanel>
    );
  }

  return (
    <SettingsPanel description={t('settings.debugMode.menuDesc')}>
      <SettingsSection>
        <DebugModeToggleRow
          field="enabled"
          label={t('settings.debugMode.enabled.label')}
          description={t('settings.debugMode.enabled.desc')}
          checked={settings.enabled}
          onChange={enabled => void update({ enabled })}
        />
        <DebugModeRootField
          key={settings.project_root ?? ''}
          value={settings.project_root}
          onCommit={project_root => void update({ project_root })}
        />
      </SettingsSection>

      <SettingsSection
        title={t('settings.debugMode.allowed.title')}
        description={t('settings.debugMode.allowed.desc')}>
        {ALWAYS_ON.map(key => (
          <SettingsRow
            key={key}
            label={t(`settings.debugMode.allowed.${key}`)}
            control={
              <SettingsBadge variant="success">{t('settings.debugMode.alwaysOn')}</SettingsBadge>
            }
          />
        ))}
        <DebugModeToggleRow
          field="auto_checkpoint"
          label={t('settings.debugMode.autoCheckpoint.label')}
          description={t('settings.debugMode.autoCheckpoint.desc')}
          checked={settings.auto_checkpoint}
          onChange={auto_checkpoint => void update({ auto_checkpoint })}
        />
        <DebugModeToggleRow
          field="auto_repair"
          label={t('settings.debugMode.autoRepair.label')}
          description={t('settings.debugMode.autoRepair.desc')}
          checked={settings.auto_repair}
          onChange={auto_repair => void update({ auto_repair })}
        />
        <DebugModeRepairsField
          key={settings.max_repair_iterations}
          value={settings.max_repair_iterations}
          onCommit={max_repair_iterations => void update({ max_repair_iterations })}
        />
        <DebugModeToggleRow
          field="run_tests_after_changes"
          label={t('settings.debugMode.runTests.label')}
          description={t('settings.debugMode.runTests.desc')}
          checked={settings.run_tests_after_changes}
          onChange={run_tests_after_changes => void update({ run_tests_after_changes })}
        />
        <DebugModeToggleRow
          field="run_build_after_changes"
          label={t('settings.debugMode.runBuild.label')}
          description={t('settings.debugMode.runBuild.desc')}
          checked={settings.run_build_after_changes}
          onChange={run_build_after_changes => void update({ run_build_after_changes })}
        />
        <DebugModeToggleRow
          field="allow_git_commit"
          label={t('settings.debugMode.gitCommit.label')}
          description={t('settings.debugMode.gitCommit.desc')}
          checked={settings.allow_git_commit}
          onChange={allow_git_commit => void update({ allow_git_commit })}
        />
      </SettingsSection>

      <SettingsSection
        title={t('settings.debugMode.dangerous.title')}
        description={t('settings.debugMode.dangerous.desc')}
        className="border-coral-300 dark:border-coral-500/40"
        data-testid="debug-dangerous-section">
        <DebugModeToggleRow
          field="allow_dependency_install"
          label={t('settings.debugMode.depInstall.label')}
          description={t('settings.debugMode.depInstall.desc')}
          checked={settings.allow_dependency_install}
          onChange={allow_dependency_install => void update({ allow_dependency_install })}
        />
        <DebugModeToggleRow
          field="allow_external_filesystem"
          label={t('settings.debugMode.externalFs.label')}
          description={t('settings.debugMode.externalFs.desc')}
          checked={settings.allow_external_filesystem}
          onChange={allow_external_filesystem => void update({ allow_external_filesystem })}
        />
        <DebugModeExternalPaths
          paths={settings.external_paths}
          onChange={external_paths => void update({ external_paths })}
        />
        <DebugModeToggleRow
          field="allow_system_commands"
          label={t('settings.debugMode.systemCommands.label')}
          description={t('settings.debugMode.systemCommands.desc')}
          checked={settings.allow_system_commands}
          onChange={allow_system_commands => void update({ allow_system_commands })}
        />
        <DebugModeToggleRow
          field="allow_git_push"
          label={t('settings.debugMode.gitPush.label')}
          description={t('settings.debugMode.gitPush.desc')}
          checked={settings.allow_git_push}
          onChange={allow_git_push => void update({ allow_git_push })}
        />
        <DebugModeToggleRow
          field="dangerous_commands_require_confirmation"
          label={t('settings.debugMode.confirmDangerous.label')}
          description={t('settings.debugMode.confirmDangerous.desc')}
          checked={settings.dangerous_commands_require_confirmation}
          onChange={dangerous_commands_require_confirmation =>
            void update({ dangerous_commands_require_confirmation })
          }
        />
        {!settings.dangerous_commands_require_confirmation && (
          <div className="px-4 py-3">
            <Alert variant="warning" data-testid="debug-confirm-off-warning">
              {t('settings.debugMode.confirmDangerous.warning')}
            </Alert>
          </div>
        )}
      </SettingsSection>

      <SettingsStatusLine
        saving={saving}
        savedNote={savedAt !== null ? t('settings.debugMode.saved') : null}
        error={saveError}
        savingLabel={t('settings.debugMode.saving')}
      />
    </SettingsPanel>
  );
};

export default DebugModePanel;
