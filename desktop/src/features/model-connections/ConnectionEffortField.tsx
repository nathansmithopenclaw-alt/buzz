import { PersonaDropdownField } from "@/features/agents/ui/PersonaDropdownField";
export type ConnectionEffort = {
  levels: readonly string[];
  value: string;
  onChange: (value: string) => void;
};
export function ConnectionEffortField({
  effort,
  disabled,
}: {
  effort: ConnectionEffort;
  disabled: boolean;
}) {
  if (effort.levels.length === 0) return null;
  return (
    <div className="space-y-1.5">
      <label className="text-sm font-medium" htmlFor="connection-effort">
        Thinking level
      </label>
      <PersonaDropdownField
        id="connection-effort"
        disabled={disabled}
        value={effort.value}
        placeholder="Choose a tested level"
        options={effort.levels.map((value) => ({ value, label: value }))}
        onValueChange={effort.onChange}
      />
      <p className="text-xs text-muted-foreground">
        Levels accepted by this model connection. Applied when the agent
        restarts.
      </p>
    </div>
  );
}
