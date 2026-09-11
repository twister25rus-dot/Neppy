import { LuCheck, LuChevronDown } from 'react-icons/lu';

import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRoot,
  DropdownMenuTrigger,
} from '../ui/DropdownMenu';

/** The six named intents, in the order the presets file lists them. */
export const PRESETS = [
  { id: 'auto', label: 'Auto', hint: 'Decides per request' },
  { id: 'fast', label: 'Fast', hint: 'Chat, simple questions, quick tasks' },
  { id: 'balanced', label: 'Balanced', hint: 'Everyday use' },
  { id: 'deep', label: 'Deep', hint: 'Coding, analysis, complex tasks' },
  { id: 'long_context', label: 'Long Context', hint: 'Large documents, long chats' },
  { id: 'maximum_quality', label: 'Maximum Quality', hint: 'When speed matters least' },
] as const;

export type PresetId = (typeof PRESETS)[number]['id'];

interface ChatPresetPillProps {
  value: PresetId;
  onChange: (next: PresetId) => void;
  className?: string;
}

/**
 * How hard the local model should work, in the chat bar.
 *
 * Wears the same shell as the Quick/Reasoning control beside it — same height,
 * radius, border and surface — so the row reads as one set of controls rather
 * than a pill next to a button next to a different button.
 *
 * A menu rather than a segmented row, which is what that control uses: six
 * options laid out side by side would be wider than the composer, and the two
 * longest names ("Long Context", "Maximum Quality") are the ones a segmented
 * row would truncate first. The trigger still shows the current choice, so the
 * state is visible without opening anything.
 *
 * Preset names are not translated. They are the vocabulary of the presets
 * document itself, and each carries a translated-looking meaning that would
 * drift from what the settings panel calls the same thing.
 */
export default function ChatPresetPill({ value, onChange, className }: ChatPresetPillProps) {
  const current = PRESETS.find(preset => preset.id === value) ?? PRESETS[0];

  return (
    <DropdownMenuRoot>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          data-analytics-id="chat-local-preset"
          data-testid="chat-preset-trigger"
          aria-label={`Model effort: ${current.label}`}
          className={`flex h-7 shrink-0 items-center gap-1 rounded-full border border-line bg-surface-subtle px-2.5 text-xs font-medium text-content-secondary transition-colors hover:text-content ${className ?? ''}`}>
          {current.label}
          <LuChevronDown aria-hidden className="h-3 w-3 text-content-faint" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-60">
        {PRESETS.map(preset => (
          <DropdownMenuItem
            key={preset.id}
            data-testid={`chat-preset-${preset.id}`}
            onSelect={() => onChange(preset.id)}
            className="flex items-start gap-2">
            <LuCheck
              aria-hidden
              className={`mt-0.5 h-3.5 w-3.5 shrink-0 ${
                preset.id === current.id ? 'text-primary-500' : 'opacity-0'
              }`}
            />
            <span className="min-w-0">
              <span className="block text-sm text-content">{preset.label}</span>
              <span className="block text-xs text-content-muted">{preset.hint}</span>
            </span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenuRoot>
  );
}
