import debug from 'debug';
import { useState } from 'react';
import { Link } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import type {
  CompanionActResult,
  CompanionSuggestion,
  SuggestionAction,
} from '../../../services/api/petCompanionApi';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import TextArea from '../../ui/TextArea';
import { formatRelative } from '../petFormat';

const log = debug('pet:companion:card');

const MAX_HANDOFF_PROMPT = 2000;
const KNOWN_KINDS = ['build_error', 'email_draft', 'term', 'ask', 'capture'];

/** Label key and analytics id for each action button, in display order. */
const ACTION_BUTTONS: Array<{ action: SuggestionAction; labelKey: string; id: string }> = [
  { action: 'explain', labelKey: 'pet.companion.action.explain', id: 'explain' },
  { action: 'draft', labelKey: 'pet.companion.action.draft', id: 'draft' },
  { action: 'copy_text', labelKey: 'pet.companion.action.copy', id: 'copy' },
  { action: 'save_note', labelKey: 'pet.companion.action.saveNote', id: 'save-note' },
  { action: 'open_chat', labelKey: 'pet.companion.action.openChat', id: 'open-chat' },
  { action: 'prepare_command', labelKey: 'pet.companion.action.prepareCommand', id: 'prepare' },
  { action: 'handoff', labelKey: 'pet.companion.action.handoff', id: 'handoff' },
];

interface CompanionSuggestionCardProps {
  suggestion: CompanionSuggestion;
  /** Runs an action against the core. Rejects on failure. */
  onAct: (action: SuggestionAction, text?: string) => Promise<CompanionActResult>;
  /** Creates a chat, records the action and navigates there with the draft seeded. */
  onOpenChat: (suggestion: CompanionSuggestion) => Promise<void>;
}

const defaultHandoffPrompt = (s: CompanionSuggestion): string =>
  [s.headline, s.body ?? s.context_excerpt ?? '']
    .filter(Boolean)
    .join('\n\n')
    .slice(0, MAX_HANDOFF_PROMPT);

/**
 * One companion suggestion. Model text is untrusted and rendered as plain text.
 * Copy writes the clipboard only from its click handler, and a hand-off shows an
 * editable prompt that is sent only after the user confirms.
 */
export default function CompanionSuggestionCard({
  suggestion,
  onAct,
  onOpenChat,
}: CompanionSuggestionCardProps) {
  const { t, locale } = useT();
  const [busy, setBusy] = useState<SuggestionAction | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [noteSaved, setNoteSaved] = useState(false);
  const [command, setCommand] = useState<string | null>(null);
  const [handoffDraft, setHandoffDraft] = useState<string | null>(null);

  const has = (action: SuggestionAction) => suggestion.actions.includes(action);
  const when = formatRelative(suggestion.created_at, locale);
  const kindLabel = KNOWN_KINDS.includes(suggestion.kind)
    ? t(`pet.companion.kind.${suggestion.kind}`)
    : suggestion.kind;

  const run = async (
    action: SuggestionAction,
    text?: string
  ): Promise<CompanionActResult | null> => {
    setBusy(action);
    setError(null);
    log('action %s id=%s', action, suggestion.id);
    try {
      return await onAct(action, text);
    } catch (err) {
      log('action %s failed id=%s err=%o', action, suggestion.id, err);
      setError(t('pet.companion.errors.actionFailed'));
      return null;
    } finally {
      setBusy(null);
    }
  };

  const handleCopy = () => {
    // Clipboard write happens synchronously inside the click, never from a timer
    // or a background event. The core is told afterwards so it can log the action.
    const text = suggestion.body;
    if (text) {
      void navigator.clipboard
        ?.writeText(text)
        .then(() => setCopied(true))
        .catch(err => log('clipboard write failed: %o', err));
    }
    void run('copy_text');
  };

  const handleClick = (action: SuggestionAction) => {
    switch (action) {
      case 'copy_text':
        handleCopy();
        return;
      case 'save_note':
        void run('save_note').then(res => res && setNoteSaved(true));
        return;
      case 'prepare_command':
        void run('prepare_command').then(res => res?.command_text && setCommand(res.command_text));
        return;
      case 'open_chat':
        setBusy('open_chat');
        setError(null);
        void onOpenChat(suggestion)
          .catch(err => {
            log('open chat failed id=%s err=%o', suggestion.id, err);
            setError(t('pet.companion.errors.actionFailed'));
          })
          .finally(() => setBusy(null));
        return;
      case 'handoff':
        setHandoffDraft(defaultHandoffPrompt(suggestion));
        return;
      default:
        void run(action);
    }
  };

  const confirmHandoff = async () => {
    const text = (handoffDraft ?? '').trim().slice(0, MAX_HANDOFF_PROMPT);
    if (!text) return;
    const res = await run('handoff', text);
    if (res) setHandoffDraft(null);
  };

  const visibleActions = ACTION_BUTTONS.filter(
    b => has(b.action) && (b.action !== 'copy_text' || Boolean(suggestion.body))
  );
  const allBusy = busy !== null;

  return (
    <li
      data-testid="companion-suggestion"
      className="rounded-2xl border border-line bg-surface p-4 shadow-subtle">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="primary">{kindLabel}</Badge>
        <Badge>{t(`pet.companion.trigger.${suggestion.trigger}`)}</Badge>
        <span className="text-xs text-content-muted">
          {t('pet.companion.card.inApp').replace('{app}', suggestion.app_name)}
          {when ? ` · ${when}` : ''}
        </span>
      </div>
      <p className="mt-2 text-sm font-medium text-content">{suggestion.headline}</p>
      {suggestion.context_excerpt && (
        <p className="mt-1 line-clamp-3 whitespace-pre-wrap rounded-lg bg-surface-subtle px-3 py-2 font-mono text-xs text-content-muted">
          {suggestion.context_excerpt}
        </p>
      )}
      {suggestion.body && (
        <p
          data-testid="companion-suggestion-body"
          className="mt-2 whitespace-pre-wrap text-sm text-content-secondary">
          {suggestion.body}
        </p>
      )}
      {command && (
        <div className="mt-2" data-testid="companion-command">
          <p className="text-xs text-content-muted">{t('pet.companion.card.commandHeading')}</p>
          <pre className="mt-1 overflow-x-auto rounded-lg bg-surface-subtle px-3 py-2 font-mono text-xs text-content">
            {command}
          </pre>
        </div>
      )}
      {suggestion.handoff && (
        <div
          data-testid="companion-handoff-status"
          className="mt-2 flex flex-wrap items-center gap-2 text-xs text-content-muted">
          <Badge
            variant={
              suggestion.handoff.status === 'done'
                ? 'success'
                : suggestion.handoff.status === 'failed'
                  ? 'danger'
                  : 'warning'
            }>
            {t(`pet.companion.handoff.status.${suggestion.handoff.status}`)}
          </Badge>
          <Link
            to={`/chat/${suggestion.handoff.thread_id}`}
            className="font-medium text-primary-600 hover:underline dark:text-primary-300">
            {t('pet.companion.handoff.openTask')}
          </Link>
          {suggestion.handoff.result_excerpt && (
            <span className="whitespace-pre-wrap">{suggestion.handoff.result_excerpt}</span>
          )}
        </div>
      )}

      {handoffDraft !== null && (
        <div className="mt-3 space-y-2" data-testid="companion-handoff-form">
          <p className="text-xs font-medium text-content">{t('pet.companion.handoff.heading')}</p>
          <p className="text-xs text-content-muted">{t('pet.companion.handoff.hint')}</p>
          <TextArea
            rows={4}
            data-testid="companion-handoff-prompt"
            aria-label={t('pet.companion.handoff.heading')}
            value={handoffDraft}
            maxLength={MAX_HANDOFF_PROMPT}
            onChange={e => setHandoffDraft(e.target.value)}
          />
          <div className="flex gap-2">
            <Button
              type="button"
              size="xs"
              analyticsId="pet-companion-handoff-confirm"
              data-testid="companion-handoff-confirm"
              disabled={allBusy || handoffDraft.trim().length === 0}
              onClick={() => void confirmHandoff()}>
              {t('pet.companion.handoff.confirm')}
            </Button>
            <Button
              type="button"
              variant="tertiary"
              size="xs"
              analyticsId="pet-companion-handoff-cancel"
              disabled={allBusy}
              onClick={() => setHandoffDraft(null)}>
              {t('common.cancel')}
            </Button>
          </div>
        </div>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2">
        {visibleActions.map(b => (
          <Button
            key={b.action}
            type="button"
            variant="secondary"
            size="xs"
            analyticsId={`pet-companion-${b.id}`}
            data-testid={`companion-action-${b.action}`}
            disabled={allBusy}
            onClick={() => handleClick(b.action)}>
            {t(b.labelKey)}
          </Button>
        ))}
        <Button
          type="button"
          variant="tertiary"
          size="xs"
          analyticsId="pet-companion-dismiss"
          data-testid="companion-action-dismiss"
          disabled={allBusy}
          onClick={() => handleClick('dismiss')}>
          {t('pet.companion.action.dismiss')}
        </Button>
        {has('mute_kind') && (
          <Button
            type="button"
            variant="tertiary"
            size="xs"
            analyticsId="pet-companion-mute-kind"
            data-testid="companion-action-mute_kind"
            disabled={allBusy}
            onClick={() => handleClick('mute_kind')}>
            {t('pet.companion.action.muteKind')}
          </Button>
        )}
        {has('mute_app') && (
          <Button
            type="button"
            variant="tertiary"
            size="xs"
            analyticsId="pet-companion-mute-app"
            data-testid="companion-action-mute_app"
            disabled={allBusy}
            onClick={() => handleClick('mute_app')}>
            {t('pet.companion.action.muteApp')}
          </Button>
        )}
      </div>
      {(copied || noteSaved) && (
        <p role="status" className="mt-2 text-xs text-sage-700 dark:text-sage-300">
          {noteSaved ? t('pet.companion.card.noteSaved') : t('pet.companion.card.copied')}
        </p>
      )}
      {error && (
        <p role="alert" className="mt-2 text-xs text-coral-600 dark:text-coral-400">
          {error}
        </p>
      )}
    </li>
  );
}
