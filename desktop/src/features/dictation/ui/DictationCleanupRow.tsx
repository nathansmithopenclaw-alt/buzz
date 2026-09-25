import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { AgentDropdownSelect } from "@/features/agents/ui/agentConfigControls";
import { listConnections } from "@/features/model-connections/api";
import { discoverAgentModels } from "@/shared/api/agentModels";
import { listSubscriptionAccounts } from "@/shared/api/tauriSubscriptionAccounts";
import { SettingsOptionRow } from "@/features/settings/ui/SettingsOptionGroup";
import { Input } from "@/shared/ui/input";
import { Button } from "@/shared/ui/button";
import type { CleanupCatalog, DictationCleanupSource } from "../api";
import { cleanupOptions, cleanupValue } from "./cleanupOptions";

export function DictationCleanupRow({
  value,
  disabled,
  catalog,
  onRetryCatalog,
  onChange,
}: {
  value: DictationCleanupSource;
  disabled: boolean;
  catalog: CleanupCatalog | undefined;
  onRetryCatalog: () => void;
  onChange: (source: DictationCleanupSource) => void;
}) {
  const connections = useQuery({
    queryKey: ["model-connections"],
    queryFn: listConnections,
  });
  const accounts = useQuery({
    queryKey: ["subscription-accounts"],
    queryFn: listSubscriptionAccounts,
  });
  const claude = useQuery({
    queryKey: ["dictation-claude-models"],
    queryFn: () => discoverAgentModels({ agentCommand: "claude" }),
    staleTime: 60_000,
  });
  const [custom, setCustom] = React.useState<Exclude<
    DictationCleanupSource,
    { kind: "off" }
  > | null>(null);
  const options = cleanupOptions(
    catalog,
    connections.data ?? [],
    claude.data?.models ?? [],
    value,
  );
  const choose = (choice: string) => {
    if (choice.startsWith("custom:")) {
      setCustom(JSON.parse(choice.slice(7)));
      return;
    }
    setCustom(null);
    onChange(JSON.parse(choice));
  };
  return (
    <SettingsOptionRow>
      <label className="font-medium" htmlFor="dictation-cleanup-model">
        Cleanup model
      </label>
      <div className="w-80 max-w-full space-y-2">
        <AgentDropdownSelect
          id="dictation-cleanup-model"
          disabled={disabled}
          searchable
          options={options}
          value={cleanupValue(value)}
          onValueChange={choose}
        />
        {custom ? (
          <div className="flex gap-2">
            <Input
              aria-label="Custom cleanup model ID"
              value={custom.model}
              onChange={(event) =>
                setCustom({ ...custom, model: event.target.value })
              }
            />
            <Button
              disabled={disabled || !custom.model.trim()}
              size="sm"
              onClick={() => {
                onChange({ ...custom, model: custom.model.trim() });
                setCustom(null);
              }}
            >
              Use model
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setCustom(null)}>
              Cancel
            </Button>
          </div>
        ) : null}
        {value.kind === "claude_plan" ? (
          <div>
            <label
              className="text-xs text-muted-foreground"
              htmlFor="dictation-claude-account"
            >
              Claude account
            </label>
            <AgentDropdownSelect
              id="dictation-claude-account"
              disabled={disabled}
              value={value.accountId ?? ""}
              onValueChange={(accountId) =>
                onChange({ ...value, accountId: accountId || null })
              }
              options={[
                { value: "", label: "Default Claude account" },
                ...(accounts.data ?? [])
                  .filter(
                    (account) =>
                      account.harness === "claude" &&
                      account.kind === "token" &&
                      account.state === "ready",
                  )
                  .map((account) => ({
                    value: account.id,
                    label: account.nickname ?? account.id,
                  })),
                ...(value.accountId &&
                !accounts.data?.some(
                  (account) =>
                    account.id === value.accountId &&
                    account.state === "ready" &&
                    account.harness === "claude" &&
                    account.kind === "token",
                )
                  ? [
                      {
                        value: value.accountId,
                        label: "Saved account (unavailable)",
                      },
                    ]
                  : []),
              ]}
            />
          </div>
        ) : null}
        {catalog?.error ? (
          <p className="text-xs text-destructive" role="alert">
            {catalog.error}
            <button
              type="button"
              className="ml-1 underline"
              onClick={onRetryCatalog}
            >
              Retry
            </button>
          </p>
        ) : null}
        {connections.error || accounts.error || claude.error ? (
          <p className="text-xs text-muted-foreground">
            Some model sources could not be loaded.{" "}
            <button
              type="button"
              className="underline"
              onClick={() => {
                void connections.refetch();
                void accounts.refetch();
                void claude.refetch();
              }}
            >
              Retry
            </button>
          </p>
        ) : null}
      </div>
    </SettingsOptionRow>
  );
}
