import { useEffect, useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';

/**
 * Manual settings for one `[[mlx.server]]` block.
 *
 * Field labels are the server's own flag names (`--kv-bits`, `--max-num-seqs`,
 * …) rendered verbatim rather than translated: they are command-line arguments
 * of an external binary, so a translated label would name a flag that exists in
 * no MLX release. Only the group headings and help text are prose.
 *
 * Values are committed on blur, not per keystroke — a field saved on every
 * character would restart the server on the way to typing "1024".
 */

export type ParamValue = string | number | boolean;

interface ParamSpec {
  /** Config field name, and the flag it becomes. */
  field: string;
  kind: 'number' | 'text' | 'bool';
  /** Shown under the field when it needs one. i18n key. */
  hintKey?: string;
  /** Only meaningful on one binary family. */
  only?: 'vlm' | 'lm';
}

interface ParamGroup {
  titleKey: string;
  params: ParamSpec[];
}

/**
 * Curated rather than exhaustive: the block carries ~40 fields and a form over
 * all of them would be unusable. These are the ones worth reaching for; the
 * rest stay editable in config.toml.
 */
const GROUPS: ParamGroup[] = [
  {
    titleKey: 'mlx.params.generation',
    params: [
      { field: 'max_tokens', kind: 'number', hintKey: 'mlx.params.zeroIsDefault' },
      { field: 'temp', kind: 'number', hintKey: 'mlx.params.negativeIsDefault', only: 'lm' },
      { field: 'top_p', kind: 'number', hintKey: 'mlx.params.negativeIsDefault', only: 'lm' },
      { field: 'top_k', kind: 'number', hintKey: 'mlx.params.negativeIsDefault', only: 'lm' },
    ],
  },
  {
    titleKey: 'mlx.params.reasoning',
    params: [
      { field: 'enable_thinking', kind: 'bool', only: 'vlm' },
      {
        field: 'thinking_budget',
        kind: 'number',
        hintKey: 'mlx.params.zeroIsDefault',
        only: 'vlm',
      },
    ],
  },
  {
    titleKey: 'mlx.params.memory',
    params: [
      { field: 'kv_bits', kind: 'number', hintKey: 'mlx.params.kvBits', only: 'vlm' },
      { field: 'max_kv_size', kind: 'number', hintKey: 'mlx.params.zeroIsDefault', only: 'vlm' },
      { field: 'max_num_seqs', kind: 'number', hintKey: 'mlx.params.maxNumSeqs', only: 'vlm' },
      { field: 'prefill_step_size', kind: 'number', hintKey: 'mlx.params.zeroIsDefault' },
    ],
  },
  {
    titleKey: 'mlx.params.speculative',
    params: [
      { field: 'draft_model', kind: 'text', hintKey: 'mlx.params.draftModel' },
      { field: 'draft_kind', kind: 'text', hintKey: 'mlx.params.draftKind', only: 'vlm' },
    ],
  },
  {
    titleKey: 'mlx.params.process',
    params: [
      { field: 'port', kind: 'number', hintKey: 'mlx.params.port' },
      { field: 'autostart', kind: 'bool' },
      { field: 'allow_lan', kind: 'bool', hintKey: 'mlx.params.allowLan' },
      { field: 'trust_remote_code', kind: 'bool', hintKey: 'mlx.params.trustRemoteCode' },
    ],
  },
];

interface Props {
  /** Current values, keyed by config field name. */
  values: Record<string, ParamValue | null | undefined>;
  isVlm: boolean;
  disabled: boolean;
  onChange: (patch: Record<string, ParamValue>) => void;
}

export default function MlxServerParams({ values, isVlm, disabled, onChange }: Props) {
  const { t } = useT();
  // Local text state so a field can be typed into without each keystroke
  // committing; committed on blur.
  const [draft, setDraft] = useState<Record<string, string>>({});

  // Drop local edits when the server's stored values change underneath, or a
  // restart would leave a stale draft sitting over the real value.
  useEffect(() => {
    setDraft({});
  }, [values]);

  const applies = (param: ParamSpec) => !param.only || (param.only === 'vlm' ? isVlm : !isVlm);

  const commitNumber = (field: string, raw: string) => {
    const trimmed = raw.trim();
    if (trimmed === '') return;
    const parsed = Number(trimmed);
    if (!Number.isFinite(parsed)) return;
    if (parsed === Number(values[field] ?? 0)) return;
    onChange({ [field]: parsed });
  };

  const commitText = (field: string, raw: string) => {
    if (raw === String(values[field] ?? '')) return;
    onChange({ [field]: raw });
  };

  return (
    <div className="flex flex-col gap-3">
      {GROUPS.map(group => {
        const params = group.params.filter(applies);
        if (params.length === 0) return null;
        return (
          <fieldset key={group.titleKey} className="flex flex-col gap-2">
            <legend className="text-xs font-medium">{t(group.titleKey)}</legend>
            {params.map(param => {
              const current = values[param.field];
              const id = `mlx-${param.field}`;
              return (
                <div key={param.field} className="flex flex-col gap-0.5">
                  <label htmlFor={id} className="flex items-center gap-2 text-sm">
                    {/* The flag name, verbatim. */}
                    <span className="min-w-44 font-mono text-xs">{param.field}</span>
                    {param.kind === 'bool' ? (
                      <input
                        id={id}
                        type="checkbox"
                        disabled={disabled}
                        checked={Boolean(current)}
                        onChange={event => onChange({ [param.field]: event.target.checked })}
                      />
                    ) : (
                      <input
                        id={id}
                        type={param.kind === 'number' ? 'number' : 'text'}
                        className="min-w-0 flex-1 rounded-md border border-border bg-surface px-2 py-1 text-sm"
                        disabled={disabled}
                        value={draft[param.field] ?? String(current ?? '')}
                        onChange={event =>
                          setDraft(prev => ({ ...prev, [param.field]: event.target.value }))
                        }
                        onBlur={event =>
                          param.kind === 'number'
                            ? commitNumber(param.field, event.target.value)
                            : commitText(param.field, event.target.value)
                        }
                      />
                    )}
                  </label>
                  {param.hintKey && (
                    <span className="pl-46 text-xs text-content-muted">{t(param.hintKey)}</span>
                  )}
                </div>
              );
            })}
          </fieldset>
        );
      })}
    </div>
  );
}
