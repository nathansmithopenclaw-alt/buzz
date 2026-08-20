import { cn } from "@/shared/lib/cn";

export type RemoteControlScopeOption = {
  id: string;
  label: string;
};

export function RemoteControlScopeSwitcher({
  onChange,
  options,
  value,
}: {
  onChange: (id: string) => void;
  options: readonly RemoteControlScopeOption[];
  value: string;
}) {
  if (options.length === 0) return null;

  return (
    <div
      className="flex flex-wrap gap-0.5 rounded-lg border border-border/70 p-0.5"
      data-testid="remote-control-scope"
      role="tablist"
      aria-label="Agent scope"
    >
      {options.map((option) => {
        const selected = option.id === value;
        return (
          <button
            aria-selected={selected}
            className={cn(
              "rounded-md px-3 py-1.5 text-xs font-medium",
              selected
                ? "bg-primary text-primary-foreground"
                : "text-muted-foreground hover:bg-muted",
            )}
            data-testid={`remote-control-scope-${option.id}`}
            key={option.id}
            onClick={() => onChange(option.id)}
            role="tab"
            type="button"
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
