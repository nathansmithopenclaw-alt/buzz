import * as React from "react";
import type {
  AcpRuntimeCatalogEntry,
  ManagedAgent,
  RuntimeConfigSurface,
} from "@/shared/api/types";
import { EffortPickerField } from "./EffortPickerField";
import {
  discoveredEffortCatalog,
  effortPickerState,
  validateSavedEffort,
} from "./effortPicker";
import { Button } from "@/shared/ui/button";

export function instanceEffortError(
  instance: ReturnType<typeof useInstanceEffort> | undefined,
  model: string,
  runtime: AcpRuntimeCatalogEntry | undefined,
  /** Discovery result as a pair: an option list is only meaningful with its configId. */
  discovered: {
    effortConfigId: string | null | undefined;
    effortOptions: readonly { id: string }[] | null;
  },
): string | null {
  if (!instance?.agent) return null;
  return validateSavedEffort(
    instance.value === undefined ? instance.agent.effortLevel : instance.value,
    instance.value !== undefined ||
      model !== (instance.agent.model ?? "") ||
      (runtime?.command != null &&
        runtime.command !== instance.agent.agentCommand),
    discoveredEffortCatalog(
      discovered.effortConfigId,
      discovered.effortOptions,
    ),
  );
}

/** Keep the authored effort draft independent of polling and discard it on reopen. */
export function useInstanceEffort(
  agent: ManagedAgent | undefined,
  open: boolean,
  config: RuntimeConfigSurface | undefined,
) {
  const [value, onChange] = React.useState<string | null | undefined>(
    undefined,
  );
  const pubkey = agent?.pubkey;
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
    effortConfigId?: string | null;
  },
) {
  if (!instance) return undefined;
  const { agent, config, value, onChange } = instance;
  if (!agent) return undefined;
  const contextChanged =
    options.model !== (agent.model ?? "") ||
    (options.runtime?.command != null &&
      options.runtime.command !== agent.agentCommand);
  const discoveredConfig = options.effortConfigId
    ? {
        effortConfigId: options.effortConfigId,
        effortOptions: options.effortOptions?.map((option) => ({
          value: option.id,
          displayName: option.name ?? option.id,
        })),
      }
    : undefined;
  const currentConfig =
    discoveredConfig ?? (contextChanged ? undefined : config);
  const visible = effortPickerState({
    backend: agent.backend,
    effortConfigId: currentConfig?.effortConfigId,
    effortOptions: currentConfig?.effortOptions,
    currentEffort: agent.effortLevel ?? null,
  }).visible;
  if (!visible) {
    return agent.backend.type === "local" && agent.effortLevel != null ? (
      <div className="space-y-1 text-xs text-muted-foreground">
        <p>
          {value === null
            ? "The saved thinking level will be cleared when you save."
            : `Saved thinking level: ${agent.effortLevel}. Live effort options are unavailable.`}
        </p>
        {value !== null ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={options.disabled}
            onClick={() => onChange(null)}
          >
            Inherit defaults
          </Button>
        ) : null}
      </div>
    ) : undefined;
  }
  return (
    <EffortPickerField
      agent={agent}
      config={currentConfig}
      disabled={
        options.disabled || (contextChanged && options.effortOptions === null)
      }
      value={value === undefined ? (agent.effortLevel ?? null) : value}
      onChange={onChange}
    />
  );
}
