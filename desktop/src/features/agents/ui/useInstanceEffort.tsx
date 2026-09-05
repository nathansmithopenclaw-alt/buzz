import * as React from "react";
import type {
  AcpRuntimeCatalogEntry,
  ManagedAgent,
  RuntimeConfigSurface,
} from "@/shared/api/types";
import { EffortPickerField } from "./EffortPickerField";
import { effortPickerState } from "./effortPicker";

/** Keep the authored effort draft independent of polling and discard it on reopen. */
export function useInstanceEffort(
  agent: ManagedAgent,
  open: boolean,
  config: RuntimeConfigSurface | undefined,
) {
  const [value, onChange] = React.useState<string | null | undefined>(
    undefined,
  );
  const pubkey = agent.pubkey;
  React.useEffect(() => {
    if (open && pubkey) onChange(undefined);
  }, [open, pubkey]);
  return { agent, config, value, onChange };
}

/** Replace the legacy control only when this instance has a canonical surface. */
export function renderInstanceEffort(
  instance: ReturnType<typeof useInstanceEffort> | undefined,
  options: {
    model: string;
    runtime: AcpRuntimeCatalogEntry | undefined;
    disabled: boolean;
    effortOptions: ReadonlyArray<{ id: string; name?: string | null }> | null;
  },
) {
  if (!instance) return undefined;
  const { agent, config, value, onChange } = instance;
  const visible = effortPickerState({
    backend: agent.backend,
    effortConfigId: config?.effortConfigId,
    effortOptions: config?.effortOptions,
    currentEffort: agent.effortLevel ?? null,
  }).visible;
  if (!visible) {
    return agent.backend.type === "local" && agent.effortLevel != null ? (
      <p className="text-xs text-muted-foreground">
        Saved reasoning effort: {agent.effortLevel}. Live effort options are
        unavailable.
      </p>
    ) : undefined;
  }
  const contextChanged =
    options.model !== (agent.model ?? "") ||
    (options.runtime?.command != null &&
      options.runtime.command !== agent.agentCommand);
  return (
    <EffortPickerField
      agent={agent}
      config={
        config && {
          ...config,
          effortOptions: contextChanged
            ? options.effortOptions?.map((option) => ({
                value: option.id,
                displayName: option.name ?? option.id,
              }))
            : config.effortOptions,
        }
      }
      disabled={
        options.disabled || (contextChanged && options.effortOptions === null)
      }
      value={value === undefined ? (agent.effortLevel ?? null) : value}
      onChange={onChange}
    />
  );
}
