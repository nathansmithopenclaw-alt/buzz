import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  deriveAgentConfigFieldModel,
  getRenderableEffortField,
} from "../lib/agentConfigCore";
import type { EnvVarsValue } from "./EnvVarsEditor";
import { getProviderEffortConfig } from "./buzzAgentConfig";
import {
  EffortSelectField,
  useEffortAutoClear,
} from "./buzzAgentModelTuningFields";
import { AgentDialogFastModeSection } from "./AgentDialogFastModeField";

export function AgentDialogEffortField({
  disabled,
  envVars,
  inheritedEnvVars = {},
  model,
  nativeEffortOptions,
  onEnvVarChange,
  provider,
  runtime,
  scope,
}: {
  disabled: boolean;
  envVars: EnvVarsValue;
  inheritedEnvVars?: EnvVarsValue;
  model: string;
  nativeEffortOptions: ReadonlyArray<{
    id: string;
    name?: string | null;
  }> | null;
  onEnvVarChange: (key: string, value: string) => void;
  provider: string;
  runtime: AcpRuntimeCatalogEntry | undefined;
  scope: "definition" | "instance";
}) {
  const config = {
    env_vars: envVars,
    model,
    provider,
  } as GlobalAgentConfig;
  const fieldModel = deriveAgentConfigFieldModel({ config, runtime, scope });
  const effortField = getRenderableEffortField(fieldModel);

  if (!effortField) {
    const omission = fieldModel.omissions.find(
      (item) => item.kind === "effort",
    );
    return (
      <ReadOnlyEffortRow
        explanation={
          omission?.reason === "ownedByModelId"
            ? "Codex owns reasoning effort in ~/.codex/config.toml (model_reasoning_effort). Buzz leaves that harness-level setting unchanged."
            : "This harness does not advertise or expose a reasoning-effort setting."
        }
      />
    );
  }

  if (
    effortField.currentPersistence.kind !== "envVar" ||
    effortField.targetApplication.kind !== "envVar"
  ) {
    return (
      <ReadOnlyEffortRow explanation="This harness owns reasoning effort outside Buzz." />
    );
  }

  const persistenceKey = effortField.currentPersistence.key;
  const currentEffort = envVars[persistenceKey] ?? "";
  const isClaudeNative = effortField.optionSource === "harnessNative";
  const table = getProviderEffortConfig(
    isClaudeNative ? "anthropic" : provider,
    model,
  );
  const nativeUnavailable = isClaudeNative && nativeEffortOptions?.length === 0;
  const availableOptions =
    isClaudeNative && nativeEffortOptions !== null
      ? nativeEffortOptions
      : undefined;
  if (nativeUnavailable) {
    return (
      <ReadOnlyEffortRow
        action={
          currentEffort
            ? {
                label: "Clear saved effort",
                onClick: () => onEnvVarChange(persistenceKey, ""),
              }
            : undefined
        }
        explanation={
          currentEffort
            ? `The selected Claude model does not advertise native effort levels, but this agent still has ${currentEffort} saved. Clear it or choose another model before saving.`
            : "The selected Claude model does not advertise native effort levels. Choose another model to enable this control."
        }
      />
    );
  }

  return (
    <RenderableEffortField
      availableOptions={availableOptions}
      disabled={disabled}
      envVars={envVars}
      inheritedEnvVars={inheritedEnvVars}
      isClaudeNative={isClaudeNative}
      nativeEffortOptions={nativeEffortOptions}
      onEnvVarChange={onEnvVarChange}
      persistenceKey={persistenceKey}
      scope={scope}
      table={table}
    />
  );
}

export function AgentDialogEffortSection({
  nativeTuning,
  onBeforeChange,
  setEnvVars,
  validationError,
  ...fieldProps
}: Omit<
  React.ComponentProps<typeof AgentDialogEffortField>,
  "envVars" | "nativeEffortOptions" | "onEnvVarChange"
> & {
  envVars: EnvVarsValue;
  nativeTuning: {
    effortOptions: ReadonlyArray<{ id: string; name?: string | null }> | null;
    fastMode: {
      configId: string | null;
      description: string | null;
      discoveryReady: boolean;
      enabled: boolean | null;
    };
  };
  onBeforeChange?: () => void;
  setEnvVars: React.Dispatch<React.SetStateAction<EnvVarsValue>>;
  validationError: string | null;
}) {
  const onEnvVarChange = (key: string, value: string) => {
    onBeforeChange?.();
    setEnvVars((current) => {
      const next = { ...current };
      if (value) next[key] = value;
      else delete next[key];
      return next;
    });
  };
  return (
    <div className="space-y-2">
      <AgentDialogEffortField
        {...fieldProps}
        envVars={fieldProps.envVars}
        nativeEffortOptions={nativeTuning.effortOptions}
        onEnvVarChange={onEnvVarChange}
      />
      {validationError ? (
        <p className="text-xs text-destructive" role="alert">
          {validationError}
        </p>
      ) : null}
      <AgentDialogFastModeSection
        disabled={fieldProps.disabled}
        discoveryReady={nativeTuning.fastMode.discoveryReady}
        envVars={fieldProps.envVars}
        model={fieldProps.model}
        nativeConfigId={nativeTuning.fastMode.configId}
        nativeDescription={nativeTuning.fastMode.description}
        nativeEnabled={nativeTuning.fastMode.enabled}
        onBeforeChange={onBeforeChange}
        runtime={fieldProps.runtime}
        scope={fieldProps.scope}
        setEnvVars={setEnvVars}
      />
    </div>
  );
}

function RenderableEffortField({
  availableOptions,
  disabled,
  envVars,
  inheritedEnvVars,
  isClaudeNative,
  nativeEffortOptions,
  onEnvVarChange,
  persistenceKey,
  scope,
  table,
}: {
  availableOptions:
    | ReadonlyArray<{ id: string; name?: string | null }>
    | undefined;
  disabled: boolean;
  envVars: EnvVarsValue;
  inheritedEnvVars: EnvVarsValue;
  isClaudeNative: boolean;
  nativeEffortOptions: ReadonlyArray<{
    id: string;
    name?: string | null;
  }> | null;
  onEnvVarChange: (key: string, value: string) => void;
  persistenceKey: string;
  scope: "definition" | "instance";
  table: ReturnType<typeof getProviderEffortConfig>;
}) {
  const validValues =
    availableOptions?.map((option) => option.id) ?? table.validValues;
  const currentEffort = envVars[persistenceKey] ?? "";
  useEffortAutoClear({
    currentEffort,
    effortValid: validValues,
    onClear: () => onEnvVarChange(persistenceKey, ""),
  });

  return (
    <div>
      <EffortSelectField
        availableOptions={availableOptions}
        currentEffort={currentEffort}
        disabled={disabled}
        effortDefault={isClaudeNative ? null : table.defaultValue}
        effortValid={validValues}
        htmlFor={`${scope}-agent-effort`}
        inheritedEffort={inheritedEnvVars[persistenceKey]}
        inheritFallbackLabel="Inherit (agent default)"
        label="Reasoning effort"
        onChange={(value) => onEnvVarChange(persistenceKey, value)}
        showUnavailableOptions={false}
        testId={`${scope}-agent-effort-select`}
        useCustomSelect
      />
      {isClaudeNative && nativeEffortOptions === null ? (
        <p className="mt-1 text-xs text-muted-foreground">
          Live Claude effort levels are unavailable; Buzz is showing the offline
          model table.
        </p>
      ) : null}
    </div>
  );
}

function ReadOnlyEffortRow({
  action,
  explanation,
}: {
  action?: { label: string; onClick: () => void };
  explanation: string;
}) {
  return (
    <div className="space-y-1.5" data-testid="agent-effort-read-only">
      <p className="text-sm font-medium text-foreground">Reasoning effort</p>
      <div className="rounded-lg border border-border bg-muted/30 px-3 py-2 text-sm text-muted-foreground">
        {explanation}
      </div>
      {action ? (
        <Button
          onClick={action.onClick}
          size="sm"
          type="button"
          variant="outline"
        >
          {action.label}
        </Button>
      ) : null}
    </div>
  );
}
