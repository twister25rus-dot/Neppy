'use client';

import { cn } from '@/components/assistant-ui/lib/utils';
import { Button } from '@/components/assistant-ui/ui/button';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/assistant-ui/ui/collapsible';
import { useT } from '@/lib/i18n/I18nContext';
import {
  type ToolApprovalOption,
  type ToolCallMessagePart,
  type ToolCallMessagePartComponent,
  type ToolCallMessagePartProps,
  type ToolCallMessagePartStatus,
  useScrollLock,
  useToolCallElapsed,
} from '@assistant-ui/react';
import {
  AlertCircleIcon,
  CheckIcon,
  ChevronDownIcon,
  ClockIcon,
  LoaderIcon,
  XCircleIcon,
} from 'lucide-react';
import { memo, useCallback, useRef, useState } from 'react';

import { type ToolApprovalRowState, useToolApprovalRowState } from './toolApprovalState';

const ANIMATION_DURATION = 200;

const pressable = 'active:scale-[0.98]';

export type ToolFallbackRootProps = Omit<
  React.ComponentProps<typeof Collapsible>,
  'open' | 'onOpenChange'
> & { open?: boolean; onOpenChange?: (open: boolean) => void; defaultOpen?: boolean };

function ToolFallbackRoot({
  className,
  open: controlledOpen,
  onOpenChange: controlledOnOpenChange,
  defaultOpen = false,
  children,
  ...props
}: ToolFallbackRootProps) {
  const collapsibleRef = useRef<HTMLDivElement>(null);
  const [uncontrolledOpen, setUncontrolledOpen] = useState(defaultOpen);
  const lockScroll = useScrollLock(collapsibleRef, ANIMATION_DURATION);

  const isControlled = controlledOpen !== undefined;
  const isOpen = isControlled ? controlledOpen : uncontrolledOpen;

  const handleOpenChange = useCallback(
    (open: boolean) => {
      lockScroll();
      if (!isControlled) {
        setUncontrolledOpen(open);
      }
      controlledOnOpenChange?.(open);
    },
    [lockScroll, isControlled, controlledOnOpenChange]
  );

  return (
    <Collapsible
      ref={collapsibleRef}
      data-slot="tool-fallback-root"
      open={isOpen}
      onOpenChange={handleOpenChange}
      className={cn('aui-tool-fallback-root group/tool-fallback-root w-full', className)}
      style={{ '--animation-duration': `${ANIMATION_DURATION}ms` } as React.CSSProperties}
      {...props}>
      {children}
    </Collapsible>
  );
}

type ToolStatus = ToolCallMessagePartStatus['type'];

const statusIconMap: Record<ToolStatus, React.ElementType> = {
  running: LoaderIcon,
  complete: CheckIcon,
  incomplete: XCircleIcon,
  'requires-action': AlertCircleIcon,
};

const formatToolDuration = (ms: number) => {
  if (ms < 1000) return '<1s';
  const seconds = ms / 1000;
  if (seconds < 10) return `${(Math.floor(seconds * 10) / 10).toFixed(1)}s`;
  if (seconds < 60) return `${Math.floor(seconds)}s`;
  return `${Math.floor(seconds / 60)}m ${Math.floor(seconds % 60)}s`;
};

function ToolFallbackDuration({ className, ...props }: React.ComponentProps<'span'>) {
  const elapsedMs = useToolCallElapsed();
  if (elapsedMs === undefined) return null;

  return (
    <span
      data-slot="tool-fallback-duration"
      className={cn(
        'aui-tool-fallback-duration text-muted-foreground text-xs tabular-nums',
        className
      )}
      {...props}>
      {formatToolDuration(elapsedMs)}
    </span>
  );
}

function ToolFallbackTrigger({
  toolName,
  status,
  approvalState,
  className,
  ...props
}: React.ComponentProps<typeof CollapsibleTrigger> & {
  toolName: string;
  status?: ToolCallMessagePartStatus;
  /** Parked on the ApprovalGate (`waiting`) or behind a parked call (`queued`). */
  approvalState?: ToolApprovalRowState;
}) {
  const statusType = status?.type ?? 'complete';
  const isWaiting = approvalState === 'waiting';
  const isQueued = approvalState === 'queued';
  // A parked or queued call is not executing, so it must not spin or shimmer.
  const isRunning = statusType === 'running' && !isWaiting && !isQueued;
  const isCancelled = status?.type === 'incomplete' && status.reason === 'cancelled';

  const { t } = useT();
  const Icon = isWaiting ? AlertCircleIcon : isQueued ? ClockIcon : statusIconMap[statusType];
  const label = isCancelled
    ? t('chat.tool.cancelled')
    : isWaiting
      ? t('chat.tool.waitingApproval')
      : isQueued
        ? t('chat.tool.queued')
        : t('chat.tool.used');

  return (
    <CollapsibleTrigger
      data-slot="tool-fallback-trigger"
      className={cn(
        'aui-tool-fallback-trigger group/trigger text-muted-foreground hover:text-foreground flex w-fit origin-left items-center gap-2 py-1.5 text-sm transition-[color,scale] active:scale-[0.98]',
        className
      )}
      {...props}>
      <Icon
        data-slot="tool-fallback-trigger-icon"
        className={cn(
          'aui-tool-fallback-trigger-icon size-4 shrink-0',
          isCancelled && 'text-muted-foreground',
          isWaiting && 'text-amber-500',
          isRunning && 'animate-spin [animation-duration:0.6s]'
        )}
      />
      <span
        data-slot="tool-fallback-trigger-label"
        data-approval-state={approvalState && approvalState !== 'none' ? approvalState : undefined}
        className={cn(
          isWaiting && 'text-amber-600 dark:text-amber-400',
          'aui-tool-fallback-trigger-label-wrapper relative inline-block text-start leading-none',
          isCancelled && 'text-muted-foreground line-through'
        )}>
        <span>
          {label}: <b>{toolName}</b>
        </span>
        {isRunning && (
          <span
            aria-hidden
            data-slot="tool-fallback-trigger-shimmer"
            className="aui-tool-fallback-trigger-shimmer shimmer pointer-events-none absolute inset-0 motion-reduce:animate-none">
            {label}: <b>{toolName}</b>
          </span>
        )}
      </span>
      <ToolFallbackDuration />
      <ChevronDownIcon
        data-slot="tool-fallback-trigger-chevron"
        className={cn(
          'aui-tool-fallback-trigger-chevron size-4 shrink-0',
          'transition-transform duration-(--animation-duration) ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none',
          // Radix Collapsible reports `data-state="open|closed"` on the trigger.
          // The `data-open` / `data-panel-open` attributes this used to key off
          // belong to Base UI and are never set here, so the chevron never moved.
          '-rotate-90',
          'group-data-[state=open]/trigger:rotate-0'
        )}
      />
    </CollapsibleTrigger>
  );
}

function ToolFallbackContent({
  className,
  children,
  ...props
}: React.ComponentProps<typeof CollapsibleContent>) {
  return (
    <CollapsibleContent
      data-slot="tool-fallback-content"
      className={cn(
        'aui-tool-fallback-content relative overflow-hidden text-sm outline-hidden',
        'group/collapsible-content ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:animate-none',
        'data-[state=closed]:animate-collapsible-up',
        'data-[state=open]:animate-collapsible-down',
        'data-[state=closed]:fill-mode-forwards',
        'data-[state=closed]:pointer-events-none',
        '[--tw-duration:var(--animation-duration)]',
        className
      )}
      {...props}>
      <div
        className={cn(
          'flex flex-col gap-2 ps-6 pt-1 pb-2 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:animate-none',
          'group-data-[state=open]/collapsible-content:animate-in group-data-[state=open]/collapsible-content:fade-in-0 group-data-[state=open]/collapsible-content:blur-in-[2px] group-data-[state=open]/collapsible-content:slide-in-from-top-1',
          'group-data-[state=closed]/collapsible-content:animate-out group-data-[state=closed]/collapsible-content:fade-out-0 group-data-[state=closed]/collapsible-content:blur-out-[2px] group-data-[state=closed]/collapsible-content:slide-out-to-top-1',
          'group-data-[state=closed]/collapsible-content:animation-duration-(--animation-duration) group-data-[state=open]/collapsible-content:animation-duration-(--animation-duration)'
        )}>
        {children}
      </div>
    </CollapsibleContent>
  );
}

/** Output longer than this is cut with a "show more" control. */
const OUTPUT_LIMIT = 4000;
/** Arguments are usually short; a long blob starts collapsed. */
const ARGS_LIMIT = 600;

/**
 * Monospace, scrollable block that truncates past `limit` characters behind a
 * "show more" button, so one huge tool result cannot swamp the transcript.
 */
function ToolFallbackText({
  text,
  limit,
  className,
}: {
  text: string;
  limit: number;
  className?: string;
}) {
  const { t } = useT();
  const [expanded, setExpanded] = useState(false);
  const overLimit = text.length > limit;
  const shown = overLimit && !expanded ? `${text.slice(0, limit)}…` : text;

  return (
    <>
      <pre
        // Scrollable regions must be reachable by keyboard.
        tabIndex={0}
        className={cn(
          'aui-tool-fallback-text bg-muted/50 text-foreground/90 mt-1 max-h-64 overflow-auto rounded-md p-2.5 font-mono text-xs wrap-break-word whitespace-pre-wrap',
          className
        )}>
        {shown}
      </pre>
      {overLimit && (
        <button
          type="button"
          aria-expanded={expanded}
          data-slot="tool-fallback-show-more"
          onClick={() => setExpanded(value => !value)}
          className="text-muted-foreground hover:text-foreground mt-1 self-start text-xs underline-offset-2 hover:underline">
          {expanded ? t('chat.tool.showLess') : t('chat.tool.showMore')}
        </button>
      )}
    </>
  );
}

function ToolFallbackArgs({
  argsText,
  className,
  ...props
}: React.ComponentProps<'div'> & { argsText?: string }) {
  const { t } = useT();
  const trimmed = argsText?.trim();
  // `{}` carries no information; an empty panel labelled "Arguments" does not
  // read as "this tool takes none", it reads as broken.
  if (!trimmed || trimmed === '{}') return null;

  return (
    <div
      data-slot="tool-fallback-args"
      className={cn('aui-tool-fallback-args flex flex-col', className)}
      {...props}>
      <p className="aui-tool-fallback-args-header text-muted-foreground text-xs font-medium">
        {t('chat.tool.arguments')}
      </p>
      <ToolFallbackText text={trimmed} limit={ARGS_LIMIT} />
    </div>
  );
}

function resultText(result: unknown): string {
  if (typeof result === 'string') return result;
  try {
    return JSON.stringify(result, null, 2) ?? String(result);
  } catch {
    return String(result);
  }
}

function ToolFallbackResult({
  result,
  isError,
  className,
  ...props
}: React.ComponentProps<'div'> & { result?: unknown; isError?: boolean }) {
  const { t } = useT();
  if (result === undefined) return null;
  const text = resultText(result);

  return (
    <div
      data-slot="tool-fallback-result"
      className={cn('aui-tool-fallback-result flex flex-col', className)}
      {...props}>
      <p
        className={cn(
          'aui-tool-fallback-result-header text-xs font-medium',
          isError ? 'text-destructive' : 'text-muted-foreground'
        )}>
        {isError ? t('chat.tool.error') : t('chat.tool.output')}
      </p>
      {text.length === 0 ? (
        <p className="text-muted-foreground mt-1 text-xs italic">{t('chat.tool.noOutput')}</p>
      ) : (
        <ToolFallbackText
          text={text}
          limit={OUTPUT_LIMIT}
          className={isError ? 'border-destructive/30 border' : undefined}
        />
      )}
    </div>
  );
}

function ToolFallbackRunning({ className, ...props }: React.ComponentProps<'p'>) {
  const { t } = useT();
  return (
    <p
      role="status"
      data-slot="tool-fallback-running"
      className={cn('aui-tool-fallback-running text-muted-foreground text-xs italic', className)}
      {...props}>
      {t('chat.tool.running')}
    </p>
  );
}

function ToolFallbackError({
  status,
  className,
  ...props
}: React.ComponentProps<'div'> & { status?: ToolCallMessagePartStatus }) {
  if (status?.type !== 'incomplete') return null;

  const error = status.error;
  const errorText = error ? (typeof error === 'string' ? error : JSON.stringify(error)) : null;

  if (!errorText) return null;

  const isCancelled = status.reason === 'cancelled';
  const headerText = isCancelled ? 'Cancelled reason:' : 'Error:';

  return (
    <div
      data-slot="tool-fallback-error"
      className={cn('aui-tool-fallback-error', className)}
      {...props}>
      <p className="aui-tool-fallback-error-header text-muted-foreground font-semibold">
        {headerText}
      </p>
      <p className="aui-tool-fallback-error-reason text-muted-foreground">{errorText}</p>
    </div>
  );
}

const APPROVED_RESULT = 'Approved by user';
const DENIED_RESULT = 'User denied tool execution';

const APPROVAL_OPTION_DEFAULT_LABELS: Record<string, string> = {
  'allow-once': 'Allow',
  'allow-always': 'Always allow',
  'reject-once': 'Deny',
  'reject-always': 'Always deny',
};

const isAllowKind = (kind: string) => kind === 'allow-once' || kind === 'allow-always';

const approvalOptionLabel = (option: ToolApprovalOption) =>
  option.label ??
  (Object.hasOwn(APPROVAL_OPTION_DEFAULT_LABELS, option.kind)
    ? APPROVAL_OPTION_DEFAULT_LABELS[option.kind]
    : undefined) ??
  option.id;

const offersInterruptAction = (
  status: ToolCallMessagePartStatus | undefined,
  approval: ToolCallMessagePart['approval'],
  interrupt: ToolCallMessagePart['interrupt']
) =>
  status?.type !== 'requires-action' ||
  status.reason !== 'interrupt' ||
  approval != null ||
  interrupt != null;

function ToolFallbackApproval({
  className,
  addResult,
  resume,
  interrupt,
  approval,
  respondToApproval,
  status,
  ...props
}: React.ComponentProps<'div'> &
  Partial<
    Pick<ToolCallMessagePartProps, 'addResult' | 'resume' | 'respondToApproval' | 'status'>
  > & {
    interrupt?: ToolCallMessagePart['interrupt'];
    approval?: ToolCallMessagePart['approval'];
  }) {
  const [submitted, setSubmitted] = useState(false);
  const [confirmingId, setConfirmingId] = useState<string | null>(null);

  if (approval != null && (approval.approved !== undefined || approval.resolution !== undefined))
    return null;

  if (!offersInterruptAction(status, approval, interrupt)) return null;

  // Custom (`_`-prefixed) kinds cannot be resolved to a boolean by the kit;
  // hosts using custom kinds render their own bar. A declared option list is
  // a host constraint: the kit never adds an approval path beyond it, but
  // always preserves a refusal path.
  const declaredOptions = respondToApproval ? approval?.options : undefined;
  const options = declaredOptions?.filter(o =>
    Object.hasOwn(APPROVAL_OPTION_DEFAULT_LABELS, o.kind)
  );

  const respond = (approved: boolean) => {
    if (submitted) return;
    if (approval != null && approval.approved === undefined && respondToApproval) {
      respondToApproval({ approved });
    } else if (interrupt) {
      resume?.({ approved });
    } else if (status?.type === 'requires-action' && status.reason === 'interrupt') {
      return;
    } else {
      addResult?.(approved ? APPROVED_RESULT : DENIED_RESULT);
    }
    setSubmitted(true);
  };

  const respondWithOption = (option: ToolApprovalOption) => {
    if (submitted) return;
    respondToApproval?.({ optionId: option.id });
    setSubmitted(true);
    setConfirmingId(null);
  };

  const handleOption = (option: ToolApprovalOption) => {
    if (option.confirm) {
      setConfirmingId(option.id);
    } else {
      respondWithOption(option);
    }
  };

  const confirming = confirmingId != null ? options?.find(o => o.id === confirmingId) : undefined;

  if (confirming) {
    const confirmMeta = typeof confirming.confirm === 'object' ? confirming.confirm : undefined;
    const confirmDescription = confirmMeta?.description ?? confirming.description;
    return (
      <div
        data-slot="tool-fallback-approval-confirm"
        className={cn('aui-tool-fallback-approval-confirm flex flex-col gap-2 pt-1', className)}
        {...props}>
        <p className="aui-tool-fallback-approval-confirm-title font-semibold">
          {confirmMeta?.title ?? `${approvalOptionLabel(confirming)}?`}
        </p>
        {confirmDescription && (
          <p className="aui-tool-fallback-approval-confirm-description text-muted-foreground">
            {confirmDescription}
          </p>
        )}
        {confirming.grants && confirming.grants.length > 0 && (
          <ul className="aui-tool-fallback-approval-confirm-grants flex flex-col gap-1">
            {confirming.grants.map(grant => (
              <li key={grant}>
                <code className="aui-tool-fallback-approval-confirm-grant bg-muted rounded px-1.5 py-0.5 text-xs">
                  {grant}
                </code>
              </li>
            ))}
          </ul>
        )}
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            className={pressable}
            onClick={() => respondWithOption(confirming)}
            disabled={submitted}>
            Confirm
          </Button>
          <Button
            size="sm"
            variant="outline"
            className={pressable}
            onClick={() => setConfirmingId(null)}
            disabled={submitted}>
            Back
          </Button>
        </div>
      </div>
    );
  }

  if (declaredOptions && declaredOptions.length > 0) {
    const allowOptions = options?.filter(o => isAllowKind(o.kind)) ?? [];
    const rejectOptions = options?.filter(o => !isAllowKind(o.kind)) ?? [];
    return (
      <div
        data-slot="tool-fallback-approval"
        className={cn(
          'aui-tool-fallback-approval flex flex-wrap items-center gap-2 pt-1',
          className
        )}
        {...props}>
        {[...allowOptions, ...rejectOptions].map(option => (
          <Button
            key={option.id}
            size="sm"
            variant={option === allowOptions[0] ? 'default' : 'outline'}
            className={pressable}
            onClick={() => handleOption(option)}
            disabled={submitted}>
            {approvalOptionLabel(option)}
          </Button>
        ))}
        {rejectOptions.length === 0 && (
          <Button
            size="sm"
            variant="outline"
            className={pressable}
            onClick={() => respond(false)}
            disabled={submitted}>
            Deny
          </Button>
        )}
      </div>
    );
  }

  return (
    <div
      data-slot="tool-fallback-approval"
      className={cn('aui-tool-fallback-approval flex items-center gap-2 pt-1', className)}
      {...props}>
      <Button size="sm" className={pressable} onClick={() => respond(true)} disabled={submitted}>
        Allow
      </Button>
      <Button
        size="sm"
        variant="outline"
        className={pressable}
        onClick={() => respond(false)}
        disabled={submitted}>
        Deny
      </Button>
    </div>
  );
}

const ToolFallbackImpl: ToolCallMessagePartComponent = ({
  toolName,
  argsText,
  result,
  status,
  addResult,
  resume,
  interrupt,
  approval,
  respondToApproval,
  isError,
}) => {
  const isCancelled = status?.type === 'incomplete' && status.reason === 'cancelled';
  const isRequiresAction = status?.type === 'requires-action';
  const approvalState = useToolApprovalRowState(toolName, status?.type === 'running');
  const shouldRenderApproval =
    isRequiresAction && offersInterruptAction(status, approval, interrupt);

  const [open, setOpen] = useState(isRequiresAction);
  const [prevRequiresAction, setPrevRequiresAction] = useState(isRequiresAction);
  if (isRequiresAction !== prevRequiresAction) {
    setPrevRequiresAction(isRequiresAction);
    if (isRequiresAction) setOpen(true);
  }

  return (
    <ToolFallbackRoot open={open} onOpenChange={setOpen}>
      <ToolFallbackTrigger toolName={toolName} status={status} approvalState={approvalState} />
      <ToolFallbackContent>
        <ToolFallbackError status={status} />
        <ToolFallbackArgs argsText={argsText} className={cn(isCancelled && 'opacity-60')} />
        {shouldRenderApproval && (
          <ToolFallbackApproval
            addResult={addResult}
            resume={resume}
            interrupt={interrupt}
            approval={approval}
            respondToApproval={respondToApproval}
            status={status}
          />
        )}
        {status?.type === 'running' && result === undefined && approvalState === 'none' && (
          <ToolFallbackRunning />
        )}
        {!isCancelled && <ToolFallbackResult result={result} isError={isError} />}
      </ToolFallbackContent>
    </ToolFallbackRoot>
  );
};

const ToolFallback = memo(ToolFallbackImpl) as unknown as ToolCallMessagePartComponent & {
  Root: typeof ToolFallbackRoot;
  Trigger: typeof ToolFallbackTrigger;
  Content: typeof ToolFallbackContent;
  Args: typeof ToolFallbackArgs;
  Result: typeof ToolFallbackResult;
  Running: typeof ToolFallbackRunning;
  Error: typeof ToolFallbackError;
  Approval: typeof ToolFallbackApproval;
};

ToolFallback.displayName = 'ToolFallback';
ToolFallback.Root = ToolFallbackRoot;
ToolFallback.Trigger = ToolFallbackTrigger;
ToolFallback.Content = ToolFallbackContent;
ToolFallback.Args = ToolFallbackArgs;
ToolFallback.Result = ToolFallbackResult;
ToolFallback.Running = ToolFallbackRunning;
ToolFallback.Error = ToolFallbackError;
ToolFallback.Approval = ToolFallbackApproval;

export {
  ToolFallback,
  ToolFallbackRoot,
  ToolFallbackTrigger,
  ToolFallbackContent,
  ToolFallbackArgs,
  ToolFallbackResult,
  ToolFallbackRunning,
  ToolFallbackError,
  ToolFallbackApproval,
};
