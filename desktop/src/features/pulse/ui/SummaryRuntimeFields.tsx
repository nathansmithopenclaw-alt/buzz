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
  const { globalConfig, inheritedDefaults, inheritedEnvVars } =
    useAgentDialogDefaults({ open: true });
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
  const providerVisible = runtimeSupportsLlmProviderSelection(
    runtime?.id ?? "",
  );
  const provider = providerVisible
    ? (providerOverride ?? inheritedDefaults.provider.value)
    : "";
  const model = modelOverride ?? inheritedDefaults.model.value;
  const mergedEnv = React.useMemo(
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
  const picker = useAgentDialogModelPicker({
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
    provider,
    envVars,
    globalEnvVars: inheritedEnvVars,
  });
  const validationError =
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
    setEnvVars((current) =>
      Object.fromEntries(
        Object.entries(current).filter(([key]) =>
          credentials.requiredEnvKeys.includes(key),
        ),
      ),
    );
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
            setRuntimeOverride(id);
            setModelOverride("");
            setProviderOverride(null);
            changeContext();
          }}
        />
      </div>
      {providerVisible ? (
        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="summary-provider">
            LLM provider
          </label>
          <PersonaDropdownField
            id="summary-provider"
            value={provider}
            disabled={disabled}
            options={getPersonaProviderOptions(provider, runtime?.id ?? "").map(
              (p) => ({ value: p.id, label: p.label }),
            )}
            onValueChange={(value) => {
              setProviderOverride(value);
              setModelOverride("");
              changeContext();
            }}
            placeholder="Choose a provider"
          />
        </div>
      ) : null}
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
      <AgentDialogEffortSection
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
