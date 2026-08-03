import * as React from "react";

import type {
  PersonaDropdownOption,
  PersonaModelOption,
} from "./agentConfigOptions";
import {
  AUTO_MODEL_DROPDOWN_VALUE,
  AUTO_PROVIDER_DROPDOWN_VALUE,
  BLOCK_BUILD_HIDDEN_PROVIDER_IDS,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
} from "./agentConfigOptions";
import { getBakedProviderInheritLabel } from "./bakedEnvHelpers";
import {
  mergeRuntimeModelCatalog,
  validateEffortSelection,
  validateModelSelection,
} from "./agentModelCatalog";
import {
  modelDropdownOptions,
  relayMeshModelPickerState,
} from "./relayMeshModelPicker";
import { MODEL_DISCOVERY_LOADING_VALUE } from "./usePersonaModelDiscovery";

type PickerInput = Omit<
  Parameters<typeof relayMeshModelPickerState>[0],
  "discoveredOptions"
> & {
  discoveredOptions: readonly PersonaModelOption[] | null;
  effort: string;
  effortProvider?: string;
  effortRuntimeId?: string;
  nativeEffortOptions: ReadonlyArray<{ id: string }> | null;
  effortModelQualifier?: boolean;
  provider: string;
  runtimeId: string;
};

export function useAgentDialogModelPicker(input: PickerInput) {
  const {
    discoveredOptions,
    effort,
    effortModelQualifier = false,
    effortProvider = input.provider,
    effortRuntimeId = input.runtimeId,
    knownOptions,
    model,
    nativeEffortOptions,
    provider,
    runtimeId,
    ...pickerInput
  } = input;
  const completeModelOptions = React.useMemo(
    () =>
      mergeRuntimeModelCatalog({
        currentModel: model,
        discovered: discoveredOptions,
        provider,
        runtimeId,
      }),
    [discoveredOptions, model, provider, runtimeId],
  );
  return {
    ...relayMeshModelPickerState({
      ...pickerInput,
      discoveredOptions: completeModelOptions,
      knownOptions: completeModelOptions.length
        ? completeModelOptions
        : knownOptions,
      model,
      provider,
    }),
    completeModelOptions,
    effortValidationError: effortModelQualifier
      ? null
      : validateEffortSelection({
          effort,
          model,
          nativeEffortOptions,
          provider: effortProvider,
          runtimeId: effortRuntimeId,
        }),
    modelValidationError: validateModelSelection(model, runtimeId, {
      enabled: effortModelQualifier,
      effortOptions: nativeEffortOptions,
    }),
  };
}

export function useHiddenProviderIds(
  bakedEnvKeys: readonly string[] | undefined,
) {
  return React.useMemo(
    () =>
      (bakedEnvKeys ?? []).includes("BUZZ_AGENT_PROVIDER")
        ? BLOCK_BUILD_HIDDEN_PROVIDER_IDS
        : new Set<string>(),
    [bakedEnvKeys],
  );
}

type ProviderOption = { id: string; label: string };

export function definitionProviderDropdownOptions(
  options: readonly ProviderOption[],
): PersonaDropdownOption[] {
  return [
    ...options
      .filter((option) => option.id.trim().length > 0)
      .map((option) => ({ label: option.label, value: option.id })),
    { label: "Custom provider...", value: CUSTOM_PROVIDER_DROPDOWN_VALUE },
  ];
}

export function instanceProviderDropdownOptions(
  options: readonly ProviderOption[],
  inheritedSource: string | null,
  inheritedValue: string | null,
): PersonaDropdownOption[] {
  return [
    ...options.map((option) => ({
      label:
        option.id === "" && inheritedSource === "build"
          ? getBakedProviderInheritLabel(inheritedValue ?? "", options)
          : option.label,
      value: option.id || AUTO_PROVIDER_DROPDOWN_VALUE,
    })),
    { label: "Custom provider...", value: CUSTOM_PROVIDER_DROPDOWN_VALUE },
  ];
}

export function definitionModelDropdownOptions(
  options: readonly PersonaModelOption[],
  isRelayMesh: boolean,
  loading: boolean,
) {
  return modelDropdownOptions({
    allowCustom: !isRelayMesh,
    globalModel: undefined,
    loading,
    loadingValue: MODEL_DISCOVERY_LOADING_VALUE,
    options,
  })
    .filter(
      (option) => isRelayMesh || option.value !== AUTO_MODEL_DROPDOWN_VALUE,
    )
    .map((option) =>
      isRelayMesh && option.value === AUTO_MODEL_DROPDOWN_VALUE
        ? { ...option, label: "Automatic" }
        : option,
    );
}

export function instanceModelDropdownOptions({
  globalModel,
  globalModelLabel,
  isRelayMesh,
  loading,
  options,
}: {
  globalModel: string;
  globalModelLabel: string;
  isRelayMesh: boolean;
  loading: boolean;
  options: readonly PersonaModelOption[];
}) {
  return modelDropdownOptions({
    allowCustom: !isRelayMesh,
    globalModel: isRelayMesh ? undefined : globalModel,
    globalModelLabel: isRelayMesh ? undefined : globalModelLabel,
    loading,
    loadingValue: MODEL_DISCOVERY_LOADING_VALUE,
    options,
  });
}
