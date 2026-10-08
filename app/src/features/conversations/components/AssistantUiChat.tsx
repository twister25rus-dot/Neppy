import { Thread, type ThreadComponents } from '@/components/assistant-ui/thread';
import { type AssistantState, useAui, useAuiState } from '@assistant-ui/react';
import debugFactory from 'debug';
import { BrainIcon, PlusIcon } from 'lucide-react';
import {
  type ChangeEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
} from 'react';

import AttachmentPreview from '../../../components/chat/AttachmentPreview';
import ChatPresetPill, { type PresetId } from '../../../components/chat/ChatPresetPill';
import {
  type ComposerEffort,
  isReasoningOn,
  REASONING_ON_EFFORT,
} from '../../../components/chat/ComposerEffortPill';
import WebSearchToggle from '../../../components/chat/WebSearchToggle';
import { Button, Switch } from '../../../components/ui';
import type { Attachment } from '../../../lib/attachments';
import { useRegisterAction } from '../../../lib/commands/useRegisterAction';
import { useSlashCommands } from '../../../lib/commands/useSlashCommands';
import { useT } from '../../../lib/i18n/I18nContext';
import { AssistantUiRuntimeProvider } from '../../../providers/AssistantUiRuntimeProvider';
import { useAppSelector } from '../../../store/hooks';
import { DEFAULT_MASCOT_COLOR } from '../../../store/mascotSlice';
import { MascotChipAvatar } from '../../human/Mascot/MascotChipAvatar';
import { SelectedThreadModeProvider } from '../threadModeContext';
import { ChatToolFallback, ChatToolGroup } from './ChatToolParts';
import { type ThreadGoalController, ThreadGoalEditorPanel } from './ThreadGoalChip';

const debug = debugFactory('assistant-ui-chat');

const selectComposerText = (state: AssistantState) => state.composer.text;

/** New-chat heading above the mode tabs and composer. */
function ChatWelcome() {
  const { t } = useT();
  return (
    <div className="mb-8 flex flex-col items-center px-4 text-center">
      <h1 className="fade-in slide-in-from-bottom-1 animate-in fill-mode-both text-4xl font-bold tracking-tight text-content duration-200">
        {t('chat.newWindowPrompt')}
      </h1>
    </div>
  );
}

function ComposerTextBridge({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const aui = useAui();
  const composerText = useAuiState(selectComposerText);
  const previousHostValue = useRef(value);

  useEffect(() => {
    // A host-side write (dictation, ESC restore, clear) wins for this pass.
    if (previousHostValue.current !== value) {
      previousHostValue.current = value;
      if (composerText !== value) aui.composer.setText(value);
      return;
    }
    // Otherwise the editor changed and the host draft follows it.
    if (composerText !== value) onChange(composerText);
  }, [aui, composerText, onChange, value]);

  return null;
}

/**
 * The assistant-ui `Thread`, projected from Neppy's Redux transcript.
 *
 * The runtime is a read-only projection; Redux and the core remain authoritative
 * for messages, streaming and persistence. Composer sends are forwarded through
 * the chat-surface registration owned by `Conversations`, so this uses the same
 * send/cancel path as the legacy composer.
 */
export function AssistantUiChat({
  threadGoal,
  model,
  onModelChange,
  sampling,
  onSamplingChange,
  composerHeader,
  inputValue,
  onInputValueChange,
  onEscape,
  attachments,
  onAttachFiles,
  onRemoveAttachment,
  maxAttachments,
  attachmentsEnabled,
  attachmentInteractionBlocked,
  onAttachmentOnlySend,
  onNeppyMode,
  preset,
  onPresetChange,
}: {
  threadGoal: ThreadGoalController;
  model: string | null;
  onModelChange: (value: string | null, contextWindow?: number | null) => void;
  /** Per-turn generation settings, and the setter that persists them. */
  sampling: ComposerEffort;
  onSamplingChange: (next: ComposerEffort) => void;
  composerHeader?: ReactNode;
  inputValue: string;
  onInputValueChange: (value: string) => void;
  onEscape?: () => void;
  attachments: Attachment[];
  onAttachFiles: (files: FileList | File[] | null) => Promise<void>;
  onRemoveAttachment: (id: string) => void;
  maxAttachments: number;
  attachmentsEnabled: boolean;
  attachmentInteractionBlocked: boolean;
  onAttachmentOnlySend: () => void;
  /** Opens the Human page from the composer's idle primary slot. */
  onNeppyMode?: () => void;
  /** Local-model effort preset shown in the composer's "Balanced" menu. */
  preset: PresetId;
  onPresetChange: (next: PresetId) => void;
}) {
  const { t } = useT();
  const fileInputRef = useRef<HTMLInputElement>(null);
  // The host passes a fresh `onAttachFiles` on every render. Read it through a
  // ref so the input's `onChange` always reaches the latest one without the
  // input itself having to be re-created when the callback changes.
  const onAttachFilesRef = useRef(onAttachFiles);
  useLayoutEffect(() => {
    onAttachFilesRef.current = onAttachFiles;
  }, [onAttachFiles]);
  const handleFileInputChange = useCallback((event: ChangeEvent<HTMLInputElement>) => {
    const files = event.target.files;
    debug('[chat][attach] file input change count=%d', files?.length ?? 0);
    void onAttachFilesRef.current(files);
    event.target.value = '';
  }, []);
  // The idle composer button wears the user's own mascot (yellow by default),
  // so the control looks like the thing it opens rather than a generic glyph.
  //
  // Read defensively rather than through `selectMascotColor` /
  // `selectCustomPrimaryColor`: this component is mounted by suites that build
  // a partial store, and those selectors dereference `state.mascot` unguarded,
  // so a store without the slice crashes the whole chat surface on render.
  // `ChatThreadView` reads `state.theme?.` the same way for the same reason.
  const mascotColor = useAppSelector(state => state.mascot?.color ?? DEFAULT_MASCOT_COLOR);
  const mascotCustomPrimary = useAppSelector(state => state.mascot?.customPrimaryColor ?? null);
  const selectedThreadId = useAppSelector(state => state.thread.selectedThreadId);
  const loadError = useAppSelector(state => state.thread.messagesError);
  const openThreadGoal = threadGoal.open;

  useRegisterAction({
    id: 'chat.goal',
    label: 'Set thread goal',
    labelKey: 'conversations.composer.command.goal',
    group: 'Chat',
    handler: openThreadGoal,
    enabled: () => selectedThreadId !== null,
    keywords: ['goal', 'objective', 'thread goal'],
    slashCommand: { id: 'goal', descriptionKey: 'conversations.composer.command.goal' },
  });
  const slashCommands = useSlashCommands();

  // Right-hand composer controls: web search, a divider, the reasoning switch
  // (on asks for `reasoning_effort: high`; off sends `off`, which the core turns
  // into `enable_thinking: false`) and the local-model effort preset menu. The thread
  // goal stays reachable through `/goal` and the command palette.
  const ComposerExtras = useCallback(
    () => (
      <>
        <WebSearchToggle />
        <span aria-hidden className="h-7 w-px shrink-0 bg-line" />
        <label
          htmlFor="chat-reasoning"
          className="flex h-10 cursor-pointer items-center gap-2.5 rounded-full border border-line px-4 text-sm font-medium text-content-secondary">
          <BrainIcon aria-hidden className="h-4.5 w-4.5 text-content-muted" />
          <span>{t('chat.agentProfile.reasoning')}</span>
          <Switch
            id="chat-reasoning"
            checked={isReasoningOn(sampling)}
            onCheckedChange={enabled =>
              onSamplingChange({ effort: enabled ? REASONING_ON_EFFORT : 'off' })
            }
            aria-label={t('chat.agentProfile.reasoning')}
          />
        </label>
        <ChatPresetPill
          value={preset}
          onChange={onPresetChange}
          className="h-10 gap-2 bg-transparent px-4 text-[15px] text-content"
          chevronClassName="h-4 w-4 text-content-muted"
        />
      </>
    ),
    [sampling, onSamplingChange, preset, onPresetChange, t]
  );
  // The goal editor belongs in the header slot, which `thread.tsx` renders
  // OUTSIDE the bordered composer shell. It used to live in `ComposerExtras`
  // under `absolute bottom-full`, and `ComposerExtras` sits in the action row
  // (`+ / model / mic`) — so "above that row" meant on top of the textarea, and
  // opening a goal covered the message you were writing. Here it flows, pushing
  // the composer down instead of covering it, and needs no positioning at all.
  const ComposerHeader = useCallback(
    () => (
      <>
        {/* Approval / plan / queue cards sit in the pinned footer. Cap them so
            a pile of parked requests scrolls inside this box instead of
            growing the footer until the composer leaves the screen. */}
        <div data-testid="composer-header-slot" className="max-h-[40vh] overflow-y-auto">
          {composerHeader}
        </div>
        <div className="pb-2">
          <ThreadGoalEditorPanel ctl={threadGoal} />
        </div>
      </>
    ),
    [composerHeader, threadGoal]
  );
  const ComposerAttachments = useCallback(
    () => (
      <AttachmentPreview
        attachments={attachments}
        onRemove={onRemoveAttachment}
        disabled={attachmentInteractionBlocked}
      />
    ),
    [attachmentInteractionBlocked, attachments, onRemoveAttachment]
  );
  // Only the "+" button lives in the slot. The hidden file input is rendered once
  // by `AssistantUiChat` itself (below): `Thread` treats a slot whose identity
  // changes as a different component type and remounts it, which would detach an
  // input whose OS file picker is still open and silently drop the chosen files.
  const ComposerAddAttachment = useCallback(
    () => (
      <Button
        type="button"
        iconOnly
        variant="secondary"
        size="xs"
        aria-label={t('composer.attachFile')}
        title={t('composer.attachFile')}
        disabled={attachmentInteractionBlocked || attachments.length >= maxAttachments}
        onClick={() => {
          debug('[chat][attach] open file picker');
          fileInputRef.current?.click();
        }}
        className="size-10 shrink-0 rounded-full border-0 bg-surface-strong p-0 text-content hover:bg-surface-hover">
        <PlusIcon className="h-5 w-5" />
      </Button>
    ),
    [attachmentInteractionBlocked, attachments.length, maxAttachments, t]
  );
  /**
   * Primary-slot control for an empty composer: a circular button carrying the
   * user's mascot, opening the Human page. Same 28px circle as the Send button
   * it stands in for, so the row's metrics don't shift when a character is
   * typed; the avatar is inset a little so the mascot reads inside the circle
   * rather than filling it edge to edge.
   */
  const ComposerIdleAction = useCallback(
    () =>
      onNeppyMode ? (
        <Button
          type="button"
          iconOnly
          variant="secondary"
          size="xs"
          analyticsId="chat-composer-human-mode"
          data-testid="composer-human-mode"
          aria-label={t('composer.humanMode')}
          title={t('composer.humanMode')}
          className="size-7 shrink-0 rounded-full p-0"
          onClick={onNeppyMode}>
          <MascotChipAvatar color={mascotColor} customPrimary={mascotCustomPrimary} size={18} />
        </Button>
      ) : null,
    [mascotColor, mascotCustomPrimary, onNeppyMode, t]
  );

  const components: ThreadComponents = useMemo(
    () => ({
      ToolFallback: ChatToolFallback,
      ToolGroup: ChatToolGroup,
      ComposerExtras,
      ComposerHeader,
      ComposerIdleAction,
      Welcome: ChatWelcome,
      ...(attachmentsEnabled
        ? {
            ComposerAttachments,
            ComposerAddAttachment,
            hasComposerAttachments: attachments.length > 0,
            onComposerAttachmentSend: onAttachmentOnlySend,
          }
        : {}),
    }),
    [
      ComposerAddAttachment,
      ComposerAttachments,
      ComposerExtras,
      ComposerHeader,
      ComposerIdleAction,
      attachments.length,
      attachmentsEnabled,
      onAttachmentOnlySend,
    ]
  );

  return (
    <AssistantUiRuntimeProvider>
      <SelectedThreadModeProvider>
        <ComposerTextBridge value={inputValue} onChange={onInputValueChange} />
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          data-testid="composer-file-input"
          onChange={handleFileInputChange}
        />
        <Thread
          components={components}
          model={model}
          onModelChange={onModelChange}
          loadError={loadError}
          onEscape={onEscape}
          slashCommands={slashCommands}
        />
      </SelectedThreadModeProvider>
    </AssistantUiRuntimeProvider>
  );
}

export default AssistantUiChat;
