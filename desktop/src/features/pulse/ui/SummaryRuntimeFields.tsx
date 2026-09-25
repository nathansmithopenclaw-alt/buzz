import { ConnectionRuntimeNotice } from "@/features/model-connections/ConnectionRuntimeNotice";
import { useConnectionModels } from "@/features/model-connections/useConnectionModels";
import { CONNECTION_ENV } from "@/features/model-connections/api";
import {
  connectionEffortError,
  connectionEffortLevels,
} from "@/features/model-connections/modelChoices";
import { selectionOnRuntimeChange } from "@/features/agents/ui/runtimeModelProviderSelection";
import { useGlobalAgentConfig } from "@/features/agents/useGlobalAgentConfig";
import { ProviderEndpointField } from "@/features/agents/ui/ProviderEndpointField";
import { useRequiredCredentialState } from "@/features/agents/ui/useRequiredCredentialState";
import { EnvVarsEditor } from "@/features/agents/ui/EnvVarsEditor";
import * as React from "react";
import { useAcpRuntimesQuery } from "@/features/agents/hooks";
import { getDefaultPersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { AgentDialogEffortSection } from "@/features/agents/ui/AgentDialogEffortField";
import { useAgentDialogDefaults } from "@/features/agents/ui/useAgentDialogDefaults";
import {
  useAgentDialogModelPicker,
  instanceModelDropdownOptions,
} from "@/features/agents/ui/useAgentDialogModelPicker";
import { usePersonaModelDiscovery } from "@/features/agents/ui/usePersonaModelDiscovery";
import { PersonaDropdownField } from "@/features/agents/ui/PersonaDropdownField";
import {
  AUTO_MODEL_DROPDOWN_VALUE,
  CUSTOM_MODEL_DROPDOWN_VALUE,
  formatRuntimeOptionLabel,
  sortPersonaRuntimes,
  runtimeSupportsLlmProviderSelection,
  getPersonaProviderOptions,
} from "@/features/agents/ui/agentConfigOptions";
import { parseModelEffortQualifier } from "@/features/agents/ui/agentModelCatalog";
import type { AcpRuntime } from "@/shared/api/types";
import { Input } from "@/shared/ui/input";

export type SummaryRuntimeDraft = {
  runtime: AcpRuntime;
  model: string;
  provider: string;
  envVars: Record<string, string>;
};

/** Uses Edit Agent's discovery, catalog merging, dropdown and native tuning controls. */
export function SummaryRuntimeFields({
  disabled,
  onChange,
}: {
  disabled: boolean;
  onChange: (draft: SummaryRuntimeDraft | null) => void;
}) {
  const runtimes = useAcpRuntimesQuery();
  const { globalConfig } = useGlobalAgentConfig();
  const [runtimeOverride, setRuntimeOverride] = React.useState<string | null>(
    null,
  );
  const [modelOverride, setModelOverride] = React.useState<string | null>(null);
  const [providerOverride, setProviderOverride] = React.useState<string | null>(
    null,
  );
  const [envVars, setEnvVars] = React.useState<Record<string, string>>({});
  const [customModel, setCustomModel] = React.useState(false);
  const available = sortPersonaRuntimes(runtimes.data ?? []).filter(
    (r): r is AcpRuntime => r.availability === "available",
  );
  const defaultRuntime =
    getDefaultPersonaRuntime(available, globalConfig.preferred_runtime) ??
    undefined;
  const runtime =
    runtimeOverride === null
      ? defaultRuntime
      : available.find((r) => r.id === runtimeOverride);
  const { inheritedDefaults, inheritedEnvVars } = useAgentDialogDefaults({
    open: true,
    runtimeId: runtime?.id ?? "",
  });
  const providerVisible = runtimeSupportsLlmProviderSelection(
    runtime?.id ?? "",
    runtime,
  );
  const provider = providerVisible
    ? (providerOverride ?? inheritedDefaults.provider.value)
    : "";
  const model = modelOverride ?? inheritedDefaults.model.value;
  const mergedEnv = React.useMemo<Record<string, string>>(
    () => ({ ...inheritedEnvVars, ...envVars }),
    [inheritedEnvVars, envVars],
  );
  const discovery = usePersonaModelDiscovery({
    envVars: mergedEnv,
    isCustomProviderEditing: false,
    model,
    modelFieldVisible: true,
    open: true,
    provider,
    selectedRuntime: runtime,
  });
  const connections = useConnectionModels(true, runtime, mergedEnv, model);
  const picker = useAgentDialogModelPicker({
    connectionOptions: connections.options,
    connectionValue: connections.value,
    discoveredOptions: discovery.discoveredModelOptions,
    effort: runtime?.effortModelQualifier
      ? (parseModelEffortQualifier(model).effort ?? "")
      : runtime?.thinkingEnvVar
        ? (envVars[runtime.thinkingEnvVar] ?? "")
        : "",
    effortModelQualifier: runtime?.effortModelQualifier ?? false,
    fallbackOptions: [],
    isCustomEditing: customModel,
    model,
    nativeEffortOptions: discovery.nativeTuning.effortOptions,
    provider,
    runtimeId: runtime?.id ?? "",
  });
  const options = instanceModelDropdownOptions({
    globalModel: inheritedDefaults.model.value,
    globalModelLabel: inheritedDefaults.model.value,
    isRelayMesh: picker.isRelayMesh,
    loading:
      discovery.modelDiscoveryLoading &&
      discovery.discoveredModelOptions === null,
    options: picker.options,
  });
  const credentials = useRequiredCredentialState({
    open: true,
    prospectiveRuntimeId: runtime?.id ?? "",
    runtime,
    provider,
    envVars,
    globalEnvVars: inheritedEnvVars,
  });
  const validationError =
    connections.error ??
    (connections.value
      ? connectionEffortError(
          connections.effortLevels,
          runtime?.thinkingEnvVar
            ? (mergedEnv[runtime.thinkingEnvVar] ?? "")
            : "",
        )
      : null) ??
    picker.modelValidationError ??
    picker.effortValidationError ??
    (credentials.requiredEnvKeyMissing
      ? "Complete the required credentials below."
      : null);
  React.useEffect(() => {
    onChange(
      runtime && !validationError && !discovery.modelDiscoveryLoading
        ? { runtime, model, provider, envVars }
        : null,
    );
  }, [
    runtime,
    model,
    provider,
    envVars,
    validationError,
    discovery.modelDiscoveryLoading,
    onChange,
  ]);
  function changeContext() {
    setEnvVars((current) => ({
      [CONNECTION_ENV]: "",
      ...Object.fromEntries(
        Object.entries(current).filter(
          ([key]) =>
            key !== runtime?.thinkingEnvVar &&
            key !== "BUZZ_ACP_FAST_MODE" &&
            key !== CONNECTION_ENV,
        ),
      ),
    }));
    setCustomModel(false);
  }
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Starts with your default harness and model. Customize them for Nectar
        here.
      </p>
      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor="summary-nectar-harness">
          Harness
        </label>
        <PersonaDropdownField
          id="summary-nectar-harness"
          disabled={disabled || runtimes.isLoading}
          value={runtime?.id ?? ""}
          placeholder="Choose a harness"
          options={available.map((r) => ({
            value: r.id,
            label: formatRuntimeOptionLabel(r),
          }))}
          onValueChange={(id) => {
            const nextRuntime = available.find((row) => row.id === id);
            const next = selectionOnRuntimeChange(
              {
                provider,
                model,
                envVars: {
                  ...envVars,
                  [CONNECTION_ENV]: mergedEnv[CONNECTION_ENV] ?? "",
                },
                isCustomModelEditing: customModel,
                isCustomProviderEditing: false,
              },
              {
                previousRuntime: runtime?.id ?? "",
                nextRuntime: id,
                nextRuntimeCanChooseProvider:
                  runtimeSupportsLlmProviderSelection(id, nextRuntime),
                nextRuntimeSupportsConnections:
                  nextRuntime?.modelConnection?.protocol != null,
                lockedRuntimeReset: "full",
              },
            );
            setRuntimeOverride(id);
            setModelOverride(next.model);
            setProviderOverride(next.provider);
            setEnvVars(next.envVars);
            setCustomModel(false);
          }}
        />
      </div>
      <ConnectionRuntimeNotice
        runtimeId={runtime?.id ?? ""}
        connection={connections.selected}
      />
      {providerVisible ? (
        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="summary-provider">
            LLM provider
          </label>
          <PersonaDropdownField
            id="summary-provider"
            value={provider}
            disabled={disabled}
            options={getPersonaProviderOptions(
              provider,
              runtime?.id ?? "",
              undefined,
              undefined,
              runtime,
            ).map((p) => ({ value: p.id, label: p.label }))}
            onValueChange={(value) => {
              setProviderOverride(value);
              setModelOverride("");
              changeContext();
            }}
            placeholder="Choose a provider"
          />
        </div>
      ) : null}
      <ProviderEndpointField
        provider={provider}
        envVars={envVars}
        inheritedEnvVars={inheritedEnvVars}
        disabled={disabled}
        onChange={(next, endpointChanged, nextProvider) => {
          if (nextProvider) setProviderOverride(nextProvider);
          setEnvVars(next);
          if (endpointChanged) {
            setModelOverride("");
            setCustomModel(false);
          }
        }}
      />
      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor="summary-nectar-model">
          Model
        </label>
        <PersonaDropdownField
          id="summary-nectar-model"
          disabled={disabled || discovery.modelDiscoveryLoading}
          value={picker.selectValue}
          options={options}
          placeholder="Default model"
          onValueChange={(value) => {
            const choice = connections.resolve(value);
            if (choice) {
              const env: Record<string, string> = {
                ...envVars,
                [CONNECTION_ENV]: choice.connection.id,
              };
              const support = runtime?.modelConnection;
              const key = runtime?.thinkingEnvVar;
              if (
                key &&
                support?.defaultEffort &&
                connectionEffortLevels(
                  choice.connection,
                  choice.model,
                  support,
                ).includes(support.defaultEffort)
              )
                env[key] = support.defaultEffort;
              delete env.BUZZ_ACP_FAST_MODE;
              setEnvVars(env);
              setModelOverride(choice.model);
              setCustomModel(false);
              return;
            }
            if (value.startsWith("saved-connection:")) return;
            changeContext();
            setCustomModel(value === CUSTOM_MODEL_DROPDOWN_VALUE);
            setModelOverride(
              value === AUTO_MODEL_DROPDOWN_VALUE
                ? null
                : value === CUSTOM_MODEL_DROPDOWN_VALUE
                  ? ""
                  : value,
            );
          }}
        />
        {picker.showCustomInput ? (
          <Input
            aria-label="Custom model ID"
            value={model}
            disabled={disabled}
            onChange={(e) => setModelOverride(e.target.value)}
          />
        ) : null}
        {discovery.modelDiscoveryStatus ? (
          <p className="text-xs text-muted-foreground">
            {discovery.modelDiscoveryStatus.message}
          </p>
        ) : null}
      </div>
      {validationError ? (
        <p role="alert" className="text-xs text-destructive">
          {validationError}
        </p>
      ) : null}
      <AgentDialogEffortSection
        connectionEffort={
          connections.value
            ? {
                levels: connections.effortLevels,
                value: runtime?.thinkingEnvVar
                  ? (mergedEnv[runtime.thinkingEnvVar] ?? "")
                  : "",
                onChange: (value) => {
                  const key = runtime?.thinkingEnvVar;
                  if (key)
                    setEnvVars((current) => ({ ...current, [key]: value }));
                },
              }
            : undefined
        }
        disabled={disabled || discovery.modelDiscoveryLoading}
        envVars={envVars}
        inheritedEnvVars={inheritedEnvVars}
        model={model}
        nativeTuning={discovery.nativeTuning}
        onModelChange={setModelOverride}
        provider={provider}
        runtime={runtime}
        scope="definition"
        setEnvVars={setEnvVars}
        validationError={validationError}
      />
      {credentials.requiredEnvKeys.length > 0 ? (
        <EnvVarsEditor
          value={envVars}
          onChange={setEnvVars}
          requiredKeys={credentials.requiredEnvKeys}
          fileSatisfiedKeys={credentials.fileSatisfiedEnvKeys}
          inheritedFrom={inheritedEnvVars}
          disabled={disabled}
        />
      ) : null}
      {runtimes.isError ? <p role="alert">Could not load harnesses.</p> : null}
    </div>
  );
}
