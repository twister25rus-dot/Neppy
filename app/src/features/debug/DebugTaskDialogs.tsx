import { useState } from 'react';

import { Button, ConfirmDialog, ModalShell, TextArea } from '../../components/ui';
import { useT } from '../../lib/i18n/I18nContext';
import type { DebugTask } from '../../services/api/debugModeApi';
import { defaultCommitMessage } from './debugFormat';

interface CommitDialogProps {
  task: DebugTask;
  busy: boolean;
  error: string | null;
  onConfirm: (message: string) => void;
  onCancel: () => void;
}

/** Commit confirmation: editable message, prefilled from the task request. */
export function CommitDialog({ task, busy, error, onConfirm, onCancel }: CommitDialogProps) {
  const { t } = useT();
  const [message, setMessage] = useState(() => defaultCommitMessage(task.request));
  const empty = message.trim().length === 0;

  return (
    <ModalShell
      title={t('debug.commit.title')}
      titleId="debug-commit-title"
      onClose={onCancel}
      closePolicy={busy ? { escape: false, backdrop: false, button: false } : undefined}
      footer={
        <div className="flex justify-end gap-2">
          <Button variant="secondary" size="sm" onClick={onCancel} disabled={busy}>
            {t('common.cancel')}
          </Button>
          <Button
            size="sm"
            analyticsId="debug-commit-confirm"
            data-testid="debug-commit-confirm"
            disabled={busy || empty}
            onClick={() => onConfirm(message.trim())}>
            {busy ? t('debug.commit.working') : t('debug.commit.confirm')}
          </Button>
        </div>
      }>
      <div className="space-y-3 text-sm text-content-secondary">
        <p>{t('debug.commit.body').replace('{count}', String(task.files_changed.length))}</p>
        <label className="block space-y-1">
          <span className="text-xs font-medium text-content">{t('debug.commit.messageLabel')}</span>
          <TextArea
            rows={3}
            value={message}
            disabled={busy}
            onChange={e => setMessage(e.target.value)}
            data-testid="debug-commit-message"
          />
        </label>
        {error ? (
          <p role="alert" className="text-coral" data-testid="debug-commit-error">
            {t('debug.commit.error').replace('{error}', error)}
          </p>
        ) : null}
      </div>
    </ModalShell>
  );
}

interface RollbackDialogProps {
  busy: boolean;
  error: string | null;
  onConfirm: () => void;
  onCancel: () => void;
}

/** Rollback confirmation; spells out that it is reversible. */
export function RollbackDialog({ busy, error, onConfirm, onCancel }: RollbackDialogProps) {
  const { t } = useT();
  return (
    <ConfirmDialog
      title={t('debug.rollback.title')}
      titleId="debug-rollback-title"
      confirmLabel={t('debug.rollback.confirm')}
      busy={busy}
      destructive
      onConfirm={onConfirm}
      onCancel={onCancel}
      body={
        <div className="space-y-2">
          <p>{t('debug.rollback.body')}</p>
          {error ? (
            <p role="alert" className="text-coral" data-testid="debug-rollback-error">
              {t('debug.rollback.error').replace('{error}', error)}
            </p>
          ) : null}
        </div>
      }
    />
  );
}
