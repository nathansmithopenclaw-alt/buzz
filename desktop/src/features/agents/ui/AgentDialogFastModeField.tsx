import * as React from "react";

import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";
import {
  deriveAgentConfigFieldModel,
  getFastModeField,
} from "../lib/agentConfigCore";
import type { EnvVarsValue } from "./EnvVarsEditor";

export function AgentDialogFastModeField({
  disabled,
  discoveryReady,
  envVars,
  model,
  nativeConfigId,
  nativeDescription,
  nativeEnabled,
  onEnvVarChange,
  runtime,
  scope,
}: {
  disabled: boolean;
  discoveryReady: boolean;
  envVars: EnvVarsValue;
  model: string;
  nativeConfigId: string | null;
  nativeDescription: string | null;
  nativeEnabled: boolean | null;
  onEnvVarChange: (key: string, value: string) => void;
  runtime: AcpRuntimeCatalogEntry | undefined;
  scope: "definition" | "instance";
}) {
  const fieldModel = deriveAgentConfigFieldModel({
    config: { env_vars: envVars, model } as GlobalAgentConfig,
    runtime,
    scope,
  });
  const field = getFastModeField(fieldModel);
  const persistenceKey = field?.currentPersistence.key ?? null;
  const persisted = persistenceKey ? envVars[persistenceKey] : undefined;
  const available = field !== undefined && nativeConfigId !== null;

  if (!available || !persistenceKey) {
    const explanation = !discoveryReady
      ? "Checking whether the selected model offers Fast mode…"
      : field
        ? "Fast mode is not available for the selected model."
        : "Fast mode is not available for this runtime.";
    return (
      <div className="space-y-1.5" data-testid="agent-fast-mode-unavailable">
        <p className="text-sm font-medium text-foreground">Fast mode</p>
        <div className="rounded-lg border border-border bg-muted/30 px-3 py-2 text-sm text-muted-foreground">
          {explanation}
        </div>
        {persistenceKey && persisted !== undefined ? (
          <Button
            onClick={() => onEnvVarChange(persistenceKey, "")}
            size="sm"
            type="button"
            variant="outline"
          >
            Clear saved Fast mode
          </Button>
        ) : null}
      </div>
    );
  }

  const enabled =
    persisted === undefined
      ? (nativeEnabled ?? false)
      : ["1", "true", "on"].includes(persisted.toLowerCase());
  const description =
    field.usageNotice === "claudeCreditsAndLimits"
      ? "Faster responses on supported Claude models. Uses usage credits at a higher rate and has separate rate limits."
      : (nativeDescription ??
        "Faster responses on supported models with increased usage.");

  return (
    <div className="space-y-1.5" data-testid="agent-fast-mode-field">
      <div className="flex items-center justify-between gap-4 rounded-lg border border-border px-3 py-2.5">
        <div className="min-w-0">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor={`${scope}-agent-fast-mode`}
          >
            Fast mode
          </label>
          <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>
        </div>
        <Switch
          checked={enabled}
          data-testid={`${scope}-agent-fast-mode-toggle`}
          disabled={disabled}
          id={`${scope}-agent-fast-mode`}
          onCheckedChange={(checked) =>
            onEnvVarChange(persistenceKey, checked ? "true" : "false")
          }
        />
      </div>
    </div>
  );
}

export function AgentDialogFastModeSection({
  envVars,
  onBeforeChange,
  setEnvVars,
  ...fieldProps
}: Omit<
  React.ComponentProps<typeof AgentDialogFastModeField>,
  "envVars" | "onEnvVarChange"
> & {
  envVars: EnvVarsValue;
  onBeforeChange?: () => void;
  setEnvVars: React.Dispatch<React.SetStateAction<EnvVarsValue>>;
}) {
  const onEnvVarChange = React.useCallback(
    (key: string, value: string) => {
      onBeforeChange?.();
      setEnvVars((current) => {
        const next = { ...current };
        if (value) next[key] = value;
        else delete next[key];
        return next;
      });
    },
    [onBeforeChange, setEnvVars],
  );

  return (
    <AgentDialogFastModeField
      {...fieldProps}
      envVars={envVars}
      onEnvVarChange={onEnvVarChange}
    />
  );
}
