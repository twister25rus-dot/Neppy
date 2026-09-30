import debug from 'debug';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { decideApproval, type PendingApproval } from '../../services/api/approvalApi';
import { decidePetProposal, type PetInbox, type PetProposal } from '../../services/api/petApi';
import { useAppDispatch } from '../../store/hooks';
import { createNewThread } from '../../store/threadSlice';
import Button from '../ui/Button';
import { ErrorBanner } from '../ui/LoadingState';
import { formatRelative } from './petFormat';

const log = debug('pet:inbox');

interface PetInboxTabProps {
  inbox: PetInbox;
  onChanged: () => void;
}

/**
 * Pet suggestions to accept or dismiss, plus approvals that background runs
 * parked. Accepting only opens a new chat with a draft in the composer: nothing
 * is ever sent from here.
 */
export default function PetInboxTab({ inbox, onChanged }: PetInboxTabProps) {
  const { t, locale } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const proposals = inbox.proposals.filter(p => p.state === 'pending');
  const approvals = inbox.approvals;

  const accept = async (proposal: PetProposal) => {
    setBusyId(proposal.id);
    setError(null);
    log('proposal accept: start id=%s', proposal.id);
    try {
      // Create the chat first: if that fails nothing has been decided yet, and
      // an empty thread left behind by a failed decide is reused by chat.
      const thread = await dispatch(createNewThread(undefined)).unwrap();
      const { chat_prompt } = await decidePetProposal(proposal.id, 'accept');
      log('proposal accept: decided id=%s seeded=%s', proposal.id, Boolean(chat_prompt));
      onChanged();
      navigate(`/chat/${thread.id}`, {
        state: { openThreadId: thread.id, ...(chat_prompt ? { composerSeed: chat_prompt } : {}) },
      });
    } catch (err) {
      log('proposal accept failed id=%s err=%o', proposal.id, err);
      setError(t('pet.errors.decideFailed'));
    } finally {
      setBusyId(null);
    }
  };

  const dismiss = async (proposal: PetProposal) => {
    setBusyId(proposal.id);
    setError(null);
    log('proposal dismiss: start id=%s', proposal.id);
    try {
      await decidePetProposal(proposal.id, 'dismiss');
      onChanged();
    } catch (err) {
      log('proposal dismiss failed id=%s err=%o', proposal.id, err);
      setError(t('pet.errors.decideFailed'));
    } finally {
      setBusyId(null);
    }
  };

  const decide = async (approval: PendingApproval, decision: 'approve_once' | 'deny') => {
    setBusyId(approval.request_id);
    setError(null);
    log('approval %s: start id=%s', decision, approval.request_id);
    try {
      await decideApproval(approval.request_id, decision);
      onChanged();
    } catch (err) {
      log('approval decide failed id=%s err=%o', approval.request_id, err);
      setError(t('pet.errors.decideFailed'));
    } finally {
      setBusyId(null);
    }
  };

  if (proposals.length === 0 && approvals.length === 0) {
    return (
      <div className="space-y-3" data-testid="pet-inbox-tab">
        {error && <ErrorBanner message={error} />}
        <div
          data-testid="pet-inbox-empty"
          className="rounded-2xl border border-dashed border-line-strong px-6 py-10 text-center">
          <h3 className="text-sm font-semibold text-content">{t('pet.inbox.emptyTitle')}</h3>
          <p className="mx-auto mt-1 max-w-md text-sm text-content-muted">
            {t('pet.inbox.emptyBody')}
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-5" data-testid="pet-inbox-tab">
      {error && <ErrorBanner message={error} />}

      {proposals.length > 0 && (
        <section className="space-y-2">
          <h3 className="text-sm font-semibold text-content">{t('pet.inbox.proposalsHeading')}</h3>
          <p className="text-xs text-content-muted">{t('pet.inbox.acceptHint')}</p>
          <ul className="space-y-2">
            {proposals.map(proposal => (
              <li
                key={proposal.id}
                data-testid="pet-proposal"
                className="rounded-2xl border border-line bg-surface p-4 shadow-subtle">
                <p className="text-sm font-medium text-content">{proposal.action_text}</p>
                <p className="mt-1 text-xs text-content-muted">
                  {t('pet.inbox.about').replace('{title}', proposal.note_title)}
                </p>
                <p className="text-xs text-content-faint">
                  {t('pet.inbox.expiresIn').replace(
                    '{when}',
                    formatRelative(proposal.expires_at, locale) ?? ''
                  )}
                </p>
                <div className="mt-3 flex flex-wrap gap-2">
                  <Button
                    type="button"
                    size="sm"
                    analyticsId="pet-proposal-accept"
                    data-testid="pet-proposal-accept"
                    disabled={busyId === proposal.id}
                    onClick={() => void accept(proposal)}>
                    {t('pet.inbox.accept')}
                  </Button>
                  <Button
                    type="button"
                    variant="tertiary"
                    size="sm"
                    analyticsId="pet-proposal-dismiss"
                    data-testid="pet-proposal-dismiss"
                    disabled={busyId === proposal.id}
                    onClick={() => void dismiss(proposal)}>
                    {t('pet.inbox.dismiss')}
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}

      {approvals.length > 0 && (
        <section className="space-y-2">
          <h3 className="text-sm font-semibold text-content">{t('pet.inbox.approvalsHeading')}</h3>
          <p className="text-xs text-content-muted">{t('pet.inbox.approvalHint')}</p>
          <ul className="space-y-2">
            {approvals.map(approval => (
              <li
                key={approval.request_id}
                data-testid="pet-approval"
                className="rounded-2xl border border-amber-200 bg-surface p-4 shadow-subtle dark:border-amber-500/30">
                <p className="font-mono text-xs text-content-secondary">{approval.tool_name}</p>
                <p className="mt-1 text-sm text-content">{approval.action_summary}</p>
                {approval.expires_at && (
                  <p className="mt-1 text-xs text-content-faint">
                    {t('pet.inbox.expiresIn').replace(
                      '{when}',
                      formatRelative(approval.expires_at, locale) ?? ''
                    )}
                  </p>
                )}
                <div className="mt-3 flex flex-wrap gap-2">
                  <Button
                    type="button"
                    size="sm"
                    analyticsId="pet-approval-approve"
                    data-testid="pet-approval-approve"
                    disabled={busyId === approval.request_id}
                    onClick={() => void decide(approval, 'approve_once')}>
                    {t('pet.inbox.approveOnce')}
                  </Button>
                  <Button
                    type="button"
                    variant="secondary"
                    tone="danger"
                    size="sm"
                    analyticsId="pet-approval-deny"
                    data-testid="pet-approval-deny"
                    disabled={busyId === approval.request_id}
                    onClick={() => void decide(approval, 'deny')}>
                    {t('pet.inbox.deny')}
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
