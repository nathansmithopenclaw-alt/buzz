import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { PersonaDropdownField } from "./PersonaDropdownField";
import {
  type PersonaDropdownOption,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";

type Props = {
  disabled: boolean;
  loading: boolean;
  modelRequired: boolean;
  model: string;
  modelSelectValue: string;
  modelDropdownOptions: readonly PersonaDropdownOption[];
  showCustomModelInput: boolean;
  modelStatusMessage: string | null;
  onModelChange: (value: string) => void;
  onCustomModelChange: (value: string) => void;
};

export function AgentInstanceModelField({
  disabled,
  loading,
  modelRequired,
  model,
  modelSelectValue,
  modelDropdownOptions,
  showCustomModelInput,
  modelStatusMessage,
  onModelChange,
  onCustomModelChange,
}: Props) {
  return (
    <div className="space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="edit-agent-model"
      >
        Model
        {modelRequired ? (
          <span className="ml-1 text-destructive" aria-hidden="true">
            *
          </span>
        ) : (
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
        )}
      </label>
      <PersonaDropdownField
        disabled={disabled || loading}
        id="edit-agent-model"
        onValueChange={onModelChange}
        options={modelDropdownOptions}
        placeholder="Default model"
        value={modelSelectValue}
      />
      {showCustomModelInput ? (
        <div
          className={cn(
            "mt-2 flex min-h-11 items-center px-3",
            PERSONA_FIELD_SHELL_CLASS,
          )}
        >
          <Input
            aria-label="Custom model ID"
            autoCorrect="off"
            className={cn(
              "h-8 px-0 py-0 leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={disabled}
            id="edit-agent-custom-model"
            onChange={(event) => onCustomModelChange(event.target.value)}
            placeholder="Custom model ID"
            value={model}
          />
        </div>
      ) : null}
      {modelStatusMessage ? (
        <p className="text-xs text-muted-foreground">{modelStatusMessage}</p>
      ) : null}
    </div>
  );
}
