import debug from 'debug';
import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  addPetGoal,
  type PetProfile,
  type PetProfilePatch,
  type PetSource,
  removePetGoal,
  type ResearchPreset,
  updatePet,
} from '../../services/api/petApi';
import Button from '../ui/Button';
import Checkbox from '../ui/Checkbox';
import Field from '../ui/Field';
import Input from '../ui/Input';
import { ErrorBanner } from '../ui/LoadingState';
import NativeSelect from '../ui/NativeSelect';
import NumberField from '../ui/NumberField';
import Switch from '../ui/Switch';
import TextArea from '../ui/TextArea';
import { isValidHHMM } from './petFormat';

const log = debug('pet:settings');

const PRESETS: ResearchPreset[] = ['light', 'standard', 'frequent'];
const SOURCES: PetSource[] = ['memory', 'tasks', 'composio', 'web'];
const MAX_GOALS = 20;
const MAX_NAME = 40;
const MAX_PERSONA = 1000;
const MAX_GOAL_TEXT = 280;
const MAX_BUDGET = 10;

interface FormState {
  name: string;
  persona: string;
  enabled: boolean;
  research_preset: ResearchPreset;
  digest_time: string;
  quiet_start: string;
  quiet_end: string;
  budget: string;
  sources: PetSource[];
}

const toForm = (pet: PetProfile): FormState => ({
  name: pet.name,
  persona: pet.persona,
  enabled: pet.enabled,
  research_preset: pet.research_preset,
  digest_time: pet.digest_time,
  quiet_start: pet.quiet_start,
  quiet_end: pet.quiet_end,
  budget: String(pet.notify_budget_per_day),
  sources: pet.sources,
});

const sameSources = (a: PetSource[], b: PetSource[]) =>
  a.length === b.length && a.every(s => b.includes(s));

/** Only the fields that differ from the saved profile. */
function buildPatch(form: FormState, pet: PetProfile): PetProfilePatch {
  const patch: PetProfilePatch = {};
  const name = form.name.trim();
  if (name !== pet.name) patch.name = name;
  if (form.persona !== pet.persona) patch.persona = form.persona;
  if (form.enabled !== pet.enabled) patch.enabled = form.enabled;
  if (form.research_preset !== pet.research_preset) patch.research_preset = form.research_preset;
  if (form.digest_time !== pet.digest_time) patch.digest_time = form.digest_time;
  if (form.quiet_start !== pet.quiet_start) patch.quiet_start = form.quiet_start;
  if (form.quiet_end !== pet.quiet_end) patch.quiet_end = form.quiet_end;
  const budget = Number.parseInt(form.budget, 10);
  if (Number.isFinite(budget) && budget !== pet.notify_budget_per_day) {
    patch.notify_budget_per_day = budget;
  }
  if (!sameSources(form.sources, pet.sources)) patch.sources = form.sources;
  return patch;
}

interface PetSettingsTabProps {
  pet: PetProfile;
  /** Called with the saved profile so the page can show it without a refetch. */
  onSaved: (pet: PetProfile) => void;
  /** Called after a goal change so the page refetches the profile. */
  onChanged: () => void;
}

/** Everything about the pet that the user can change. */
export default function PetSettingsTab({ pet, onSaved, onChanged }: PetSettingsTabProps) {
  const { t } = useT();
  const [form, setForm] = useState<FormState>(() => toForm(pet));
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [goalText, setGoalText] = useState('');
  const [goalBusy, setGoalBusy] = useState(false);
  const [goalError, setGoalError] = useState<string | null>(null);

  const patch = buildPatch(form, pet);
  const dirty = Object.keys(patch).length > 0;

  const update = <K extends keyof FormState>(key: K, value: FormState[K]) => {
    setForm(prev => ({ ...prev, [key]: value }));
    setSaved(false);
  };

  const commitBudget = () => {
    const n = Number.parseInt(form.budget, 10);
    const clamped = Number.isFinite(n) ? Math.min(Math.max(n, 0), MAX_BUDGET) : 0;
    update('budget', String(clamped));
  };

  const toggleSource = (source: PetSource, on: boolean) => {
    update(
      'sources',
      on
        ? [...form.sources.filter(s => s !== source), source]
        : form.sources.filter(s => s !== source)
    );
  };

  const validate = (): string | null => {
    const name = form.name.trim();
    if (name.length < 1 || name.length > MAX_NAME) return t('pet.errors.nameRequired');
    if (
      !isValidHHMM(form.digest_time) ||
      !isValidHHMM(form.quiet_start) ||
      !isValidHHMM(form.quiet_end)
    ) {
      return t('pet.errors.invalidTime');
    }
    return null;
  };

  const handleSave = async () => {
    setSaved(false);
    const problem = validate();
    if (problem) {
      setError(problem);
      return;
    }
    if (!dirty) return;
    setSaving(true);
    setError(null);
    log('save fields=%s', Object.keys(patch).join(','));
    try {
      const next = await updatePet(patch);
      setForm(toForm(next));
      setSaved(true);
      onSaved(next);
    } catch (err) {
      log('save failed: %o', err);
      setError(t('pet.errors.saveFailed'));
    } finally {
      setSaving(false);
    }
  };

  const handleAddGoal = async () => {
    const text = goalText.trim();
    if (!text) return;
    setGoalBusy(true);
    setGoalError(null);
    log('goal add');
    try {
      await addPetGoal(text);
      setGoalText('');
      onChanged();
    } catch (err) {
      log('goal add failed: %o', err);
      setGoalError(t('pet.errors.goalFailed'));
    } finally {
      setGoalBusy(false);
    }
  };

  const handleRemoveGoal = async (goalId: string) => {
    setGoalBusy(true);
    setGoalError(null);
    log('goal remove id=%s', goalId);
    try {
      await removePetGoal(goalId);
      onChanged();
    } catch (err) {
      log('goal remove failed id=%s err=%o', goalId, err);
      setGoalError(t('pet.errors.goalFailed'));
    } finally {
      setGoalBusy(false);
    }
  };

  const goalsFull = pet.goals.length >= MAX_GOALS;

  return (
    <div className="space-y-5" data-testid="pet-settings-tab">
      <section className="divide-y divide-line overflow-hidden rounded-2xl border border-line bg-surface shadow-subtle">
        <Field
          htmlFor="pet-enabled"
          label={t('pet.settings.enabled')}
          description={t('pet.settings.enabledHint')}
          control={
            <Switch
              id="pet-enabled"
              data-testid="pet-enabled-switch"
              checked={form.enabled}
              onCheckedChange={v => update('enabled', v)}
            />
          }
        />
        <Field
          stacked
          htmlFor="pet-name"
          label={t('pet.settings.name')}
          control={
            <Input
              id="pet-name"
              data-testid="pet-name-input"
              value={form.name}
              maxLength={MAX_NAME}
              placeholder={t('pet.settings.namePlaceholder')}
              onChange={e => update('name', e.target.value)}
            />
          }
        />
        <Field
          stacked
          htmlFor="pet-persona"
          label={t('pet.settings.persona')}
          control={
            <TextArea
              id="pet-persona"
              rows={3}
              value={form.persona}
              maxLength={MAX_PERSONA}
              placeholder={t('pet.settings.personaPlaceholder')}
              onChange={e => update('persona', e.target.value)}
            />
          }
        />
        <Field
          stacked
          htmlFor="pet-preset"
          label={t('pet.settings.preset')}
          description={t(`pet.settings.presetDesc.${form.research_preset}`)}
          control={
            <NativeSelect
              id="pet-preset"
              data-testid="pet-preset-select"
              value={form.research_preset}
              onChange={e => update('research_preset', e.target.value as ResearchPreset)}>
              {PRESETS.map(p => (
                <option key={p} value={p}>
                  {t(`pet.settings.presetOption.${p}`)}
                </option>
              ))}
            </NativeSelect>
          }
        />
        <Field
          stacked
          htmlFor="pet-digest-time"
          label={t('pet.settings.digestTime')}
          control={
            <Input
              id="pet-digest-time"
              data-testid="pet-digest-time-input"
              type="time"
              className="w-32"
              value={form.digest_time}
              onChange={e => update('digest_time', e.target.value)}
            />
          }
        />
        <Field
          stacked
          label={t('pet.settings.quietHours')}
          description={t('pet.settings.quietHoursHint')}
          control={
            <div className="flex flex-wrap items-center gap-3">
              <label className="flex items-center gap-2 text-xs text-content-muted">
                {t('pet.settings.quietStart')}
                <Input
                  type="time"
                  data-testid="pet-quiet-start-input"
                  className="w-28"
                  value={form.quiet_start}
                  onChange={e => update('quiet_start', e.target.value)}
                />
              </label>
              <label className="flex items-center gap-2 text-xs text-content-muted">
                {t('pet.settings.quietEnd')}
                <Input
                  type="time"
                  data-testid="pet-quiet-end-input"
                  className="w-28"
                  value={form.quiet_end}
                  onChange={e => update('quiet_end', e.target.value)}
                />
              </label>
            </div>
          }
        />
        <Field
          stacked
          htmlFor="pet-budget"
          label={t('pet.settings.notifyBudget')}
          description={t('pet.settings.notifyBudgetHint')}
          control={
            <NumberField
              id="pet-budget"
              data-testid="pet-budget-field"
              aria-label={t('pet.settings.notifyBudget')}
              value={form.budget}
              min={0}
              max={MAX_BUDGET}
              onChange={v => update('budget', v)}
              onCommit={commitBudget}
            />
          }
        />
        <Field
          stacked
          label={t('pet.settings.sources')}
          description={t('pet.settings.sourcesHint')}
          control={
            <div className="space-y-2">
              {SOURCES.map(source => (
                <label key={source} className="flex items-center gap-2 text-sm text-content">
                  <Checkbox
                    data-testid={`pet-source-${source}`}
                    checked={form.sources.includes(source)}
                    onCheckedChange={on => toggleSource(source, on)}
                  />
                  {t(`pet.settings.source.${source}`)}
                </label>
              ))}
            </div>
          }
        />
      </section>

      <div className="space-y-2">
        <p className="text-xs text-content-muted">{t('pet.settings.awakeNote')}</p>
        {error && <ErrorBanner message={error} />}
        <div className="flex items-center gap-3">
          <Button
            type="button"
            size="sm"
            analyticsId="pet-settings-save"
            data-testid="pet-save"
            disabled={saving || !dirty}
            onClick={() => void handleSave()}>
            {t('pet.settings.save')}
          </Button>
          {saved && (
            <span role="status" className="text-xs text-sage-700 dark:text-sage-300">
              {t('pet.settings.saved')}
            </span>
          )}
        </div>
      </div>

      <section className="space-y-2 rounded-2xl border border-line bg-surface p-4 shadow-subtle">
        <h3 className="text-sm font-semibold text-content">{t('pet.settings.goals')}</h3>
        <p className="text-xs text-content-muted">{t('pet.settings.goalsHint')}</p>
        {pet.goals.length === 0 ? (
          <p className="text-xs italic text-content-faint">{t('pet.settings.goalsEmpty')}</p>
        ) : (
          <ul className="space-y-1.5" data-testid="pet-goals">
            {pet.goals.map(goal => (
              <li
                key={goal.id}
                data-testid="pet-goal"
                className="flex items-center justify-between gap-3 rounded-lg bg-surface-subtle px-3 py-1.5 text-sm text-content">
                <span className="min-w-0 wrap-break-word">{goal.text}</span>
                <Button
                  type="button"
                  variant="tertiary"
                  tone="danger"
                  size="xs"
                  analyticsId="pet-goal-remove"
                  aria-label={t('pet.settings.removeGoal')}
                  disabled={goalBusy}
                  onClick={() => void handleRemoveGoal(goal.id)}>
                  {t('pet.settings.removeGoal')}
                </Button>
              </li>
            ))}
          </ul>
        )}
        {goalsFull && <p className="text-xs text-content-muted">{t('pet.settings.goalsFull')}</p>}
        {goalError && <ErrorBanner message={goalError} />}
        <div className="flex gap-2">
          <Input
            data-testid="pet-goal-input"
            aria-label={t('pet.settings.goalPlaceholder')}
            placeholder={t('pet.settings.goalPlaceholder')}
            value={goalText}
            maxLength={MAX_GOAL_TEXT}
            disabled={goalsFull}
            onChange={e => setGoalText(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter') {
                e.preventDefault();
                void handleAddGoal();
              }
            }}
          />
          <Button
            type="button"
            variant="secondary"
            size="md"
            analyticsId="pet-goal-add"
            data-testid="pet-goal-add"
            disabled={goalBusy || goalsFull || goalText.trim().length === 0}
            onClick={() => void handleAddGoal()}>
            {t('pet.settings.addGoal')}
          </Button>
        </div>
      </section>
    </div>
  );
}
