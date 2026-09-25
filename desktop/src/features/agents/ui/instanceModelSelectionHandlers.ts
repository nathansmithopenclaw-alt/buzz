import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import type { useConnectionModels } from "@/features/model-connections/useConnectionModels";
import { connectionEffortLevels } from "@/features/model-connections/modelChoices";
import type { useInstanceEffort } from "./useInstanceEffort";
import { AUTO_PROVIDER_DROPDOWN_VALUE } from "./agentConfigOptions";
import {
  type RuntimeModelProviderSelection,
  selectionOnConnectionModel,
  selectionOnModelDropdownChange,
  selectionOnProviderDropdownChange,
} from "./runtimeModelProviderSelection";

/** Keep the instance editor's model/source transition and effort draft together. */
export function instanceModelSelectionHandlers({
  selection,
  applySelection,
  handleRuntimeDropdownChange,
  selectedRuntime,
  selectedRuntimeId,
  connections,
  instanceEffort,
  savedEffort,
}: {
  selection: RuntimeModelProviderSelection;
  applySelection: (next: RuntimeModelProviderSelection) => void;
  handleRuntimeDropdownChange: (value: string) => void;
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
  selectedRuntimeId: string;
  connections: ReturnType<typeof useConnectionModels>;
  instanceEffort: ReturnType<typeof useInstanceEffort>;
  savedEffort: string | null | undefined;
}) {
  return {
    handleProviderDropdownChange(nextValue: string) {
      const nextProvider =
        nextValue === AUTO_PROVIDER_DROPDOWN_VALUE ? "" : nextValue;
      if (nextProvider === "relay-mesh" && selectedRuntimeId !== "buzz-agent")
        handleRuntimeDropdownChange("buzz-agent");
      const next = selectionOnProviderDropdownChange(selection, {
        runtime:
          nextProvider === "relay-mesh"
            ? "buzz-agent"
            : (selectedRuntime?.id ?? selectedRuntimeId),
        nextValue,
        clearModelWhenApiKeyMissing: false,
      });
      applySelection({
        ...next,
        model: nextProvider === "relay-mesh" ? "auto" : next.model,
      });
    },
    handleModelDropdownChange(nextValue: string) {
      const source = connections.resolve(nextValue);
      if (source) {
        const support = selectedRuntime?.modelConnection;
        const initialEffort = support?.defaultEffort;
        if (
          support &&
          initialEffort &&
          connectionEffortLevels(
            source.connection,
            source.model,
            support,
          ).includes(initialEffort)
        )
          instanceEffort.onChange(initialEffort);
        applySelection(
          selectionOnConnectionModel(
            selection,
            source.connection.id,
            source.model,
          ),
        );
        return;
      }
      if (nextValue.startsWith("saved-connection:")) return;
      applySelection(
        selectionOnModelDropdownChange(selection, {
          nextValue,
          clearKnownModelOnCustomEntry: false,
          isModelCustom: false,
          effortModelQualifier:
            selectedRuntime?.effortModelQualifier &&
            savedEffort == null &&
            instanceEffort.value == null,
        }),
      );
    },
  };
}
