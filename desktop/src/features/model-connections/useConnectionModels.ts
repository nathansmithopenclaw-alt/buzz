import * as React from "react";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import {
  CONNECTION_ENV,
  listConnections,
  codexWorkspaceReason,
  subscribeConnections,
  type ModelConnection,
} from "./api";
import {
  usesConnectionProvider,
  connectionEffortLevels,
  connectionSelectionError,
  connectionModelChoices,
  connectionModelValue,
  resolveConnectionModelValue,
} from "./modelChoices";

/** Saved metadata only: opening a model picker never tests a server. */
export function useConnectionModels(
  open: boolean,
  runtime: AcpRuntimeCatalogEntry | undefined,
  envVars: Record<string, string>,
  model: string,
  selectionBlockReason?: string | null,
) {
  const [loading, setLoading] = React.useState(true);
  const [rows, setRows] = React.useState<ModelConnection[]>([]);
  const [workspaceReason, setWorkspaceReason] = React.useState<string | null>(
    null,
  );
  const [error, setError] = React.useState<string | null>(null);
  const source = envVars[CONNECTION_ENV]?.trim();
  const legacyProvider = usesConnectionProvider(runtime);
  React.useEffect(() => {
    if (!open) return;
    let live = true;
    let revision = 0;
    const refresh = () => {
      const current = ++revision;
      setLoading(true);
      Promise.all([
        listConnections(),
        runtime?.id === "codex"
          ? codexWorkspaceReason()
          : Promise.resolve(null),
      ])
        .then(([value, reason]) => {
          if (live && current === revision) {
            setRows(value);
            setWorkspaceReason(reason);
            setError(null);
          }
        })
        .catch(() => {
          if (live && current === revision)
            setError(
              "Saved model connections are unavailable. Reopen this dialog to try again.",
            );
        })
        .finally(() => {
          if (live && current === revision) setLoading(false);
        });
    };
    const unsubscribe = subscribeConnections(refresh);
    refresh();
    return () => {
      live = false;
      unsubscribe();
    };
  }, [open, runtime?.id]);
  const options = connectionModelChoices(
    rows,
    legacyProvider ? null : runtime?.modelConnection,
  ).map((row) => {
    const selection = resolveConnectionModelValue(row.value, rows);
    const context = selection?.connection.qualification?.context_window;
    const contextReason =
      runtime?.id === "codex" && (!context || context < 1024)
        ? "Test this connection to establish its context window"
        : null;
    const reason = selectionBlockReason ?? workspaceReason ?? contextReason;
    return reason ? { ...row, disabled: true, badge: reason } : row;
  });
  const value =
    source && !legacyProvider ? connectionModelValue(source, model) : null;
  const selected = source ? rows.find((row) => row.id === source) : undefined;
  return {
    effortLevels:
      selected && runtime?.modelConnection
        ? connectionEffortLevels(selected, model, runtime.modelConnection)
        : [],
    rows,
    options,
    value,
    selected,
    error: connectionSelectionError(
      value,
      options,
      loading,
      error,
      selected?.name,
    ),
    loading,
    resolve: (choice: string) => {
      if (options.find((option) => option.value === choice)?.disabled !== false)
        return null;
      return resolveConnectionModelValue(choice, rows);
    },
  };
}
