import { SettingsRow, SettingsSwitch } from '../controls';

interface DebugModeToggleRowProps {
  /** Settings field name; becomes the switch id and analytics id suffix. */
  field: string;
  label: string;
  description?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
}

/** One labelled switch row with a content-free analytics id. */
const DebugModeToggleRow = ({
  field,
  label,
  description,
  checked,
  onChange,
}: DebugModeToggleRowProps) => (
  <SettingsRow
    htmlFor={`switch-debug-${field}`}
    label={label}
    description={description}
    control={
      <span data-analytics-id={`settings-debug-mode-toggle-${field}`}>
        <SettingsSwitch
          id={`switch-debug-${field}`}
          checked={checked}
          onCheckedChange={onChange}
          aria-label={label}
        />
      </span>
    }
  />
);

export default DebugModeToggleRow;
