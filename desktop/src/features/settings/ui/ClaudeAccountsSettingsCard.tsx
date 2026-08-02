import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  AlertTriangle,
  CheckCircle2,
  LoaderCircle,
  Trash2,
} from "lucide-react";

import {
  addClaudeAccount,
  deleteClaudeAccount,
  type ClaudeAccount,
} from "@/shared/api/tauriClaudeAccounts";
import {
  claudeAccountsQueryKey,
  useClaudeAccountsQuery,
} from "@/features/agents/claudeAccounts";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SectionHeader } from "@/shared/ui/PageHeader";

function formatPercent(value: number | null | undefined) {
  return value == null ? "—" : `${Math.round(value)}%`;
}

function formatReset(value: string | null | undefined) {
  if (!value) return "reset unavailable";
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : `resets ${date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`;
}

function AccountStatus({ account }: { account: ClaudeAccount }) {
  if (account.usageStatus === "scope_insufficient") {
    return (
      <p className="mt-2 flex items-start gap-1.5 text-xs text-amber-600 dark:text-amber-400">
        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        Usage meter needs a login-minted token; re-mint with interactive login.
      </p>
    );
  }
  if (account.usageStatus === "stale") {
    return (
      <p className="mt-2 text-xs text-muted-foreground">
        Usage is rate limited. Meter checked{" "}
        {account.usageCheckedAt
          ? new Date(account.usageCheckedAt).toLocaleString()
          : "just now"}
        ; retrying on refresh.
      </p>
    );
  }
  if (account.usageStatus === "unavailable") {
    return (
      <p className="mt-2 text-xs text-muted-foreground">
        Usage data unavailable
        {account.usageHttpStatus ? ` (HTTP ${account.usageHttpStatus})` : ""}.
      </p>
    );
  }
  return (
    <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
      <span>
        5-hour{" "}
        <strong className="text-foreground">
          {formatPercent(account.fiveHour?.utilization)}
        </strong>{" "}
        · {formatReset(account.fiveHour?.resetsAt)}
      </span>
      <span>
        Weekly{" "}
        <strong className="text-foreground">
          {formatPercent(account.weekly?.utilization)}
        </strong>{" "}
        · {formatReset(account.weekly?.resetsAt)}
      </span>
    </div>
  );
}

export function ClaudeAccountsSettingsCard() {
  const queryClient = useQueryClient();
  const accountsQuery = useClaudeAccountsQuery();
  const [label, setLabel] = React.useState("");
  const [token, setToken] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const addMutation = useMutation({
    mutationFn: () => addClaudeAccount(label, token),
    onSuccess: async () => {
      setLabel("");
      setToken("");
      setError(null);
      await queryClient.invalidateQueries({ queryKey: claudeAccountsQueryKey });
    },
    onError: (cause) => setError(String(cause)),
  });
  const deleteMutation = useMutation({
    mutationFn: deleteClaudeAccount,
    onSuccess: async () =>
      queryClient.invalidateQueries({ queryKey: claudeAccountsQueryKey }),
    onError: (cause) => setError(String(cause)),
  });

  return (
    <section
      className="min-w-0 space-y-4"
      data-testid="settings-claude-accounts"
    >
      <SectionHeader
        title="Claude Accounts"
        description="Save Claude OAuth accounts in your system keychain, see live limits when the token supports them, and switch agents without pasting again."
      />
      <div className="space-y-3 rounded-xl border border-border/60 bg-card/50 p-4">
        {accountsQuery.isPending ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <LoaderCircle className="h-4 w-4 animate-spin" /> Loading accounts…
          </div>
        ) : accountsQuery.isError ? (
          <p className="text-sm text-destructive">
            Could not load Claude accounts: {String(accountsQuery.error)}
          </p>
        ) : accountsQuery.data.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No Claude accounts saved yet.
          </p>
        ) : (
          accountsQuery.data.map((account) => (
            <div
              className="rounded-lg border border-border/60 bg-background/60 p-3"
              data-testid={`claude-account-${account.id}`}
              key={account.id}
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <p className="font-medium">{account.label}</p>
                    {account.planType ? (
                      <span className="rounded-full bg-muted px-2 py-0.5 text-xs uppercase">
                        {account.planType}
                      </span>
                    ) : null}
                    {account.usageStatus === "available" ? (
                      <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />
                    ) : null}
                  </div>
                  <p className="mt-0.5 text-xs text-muted-foreground">
                    sha256 {account.fingerprint} · {account.length} chars
                  </p>
                </div>
                <Button
                  aria-label={`Delete ${account.label}`}
                  disabled={deleteMutation.isPending}
                  onClick={() => deleteMutation.mutate(account.id)}
                  size="icon"
                  variant="ghost"
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
              <AccountStatus account={account} />
            </div>
          ))
        )}
      </div>
      <form
        className="space-y-3 rounded-xl border border-border/60 p-4"
        onSubmit={(event) => {
          event.preventDefault();
          setError(null);
          addMutation.mutate();
        }}
      >
        <p className="text-sm font-medium">Add account</p>
        <Input
          aria-label="Account label"
          onChange={(event) => setLabel(event.target.value)}
          placeholder="Account A"
          value={label}
        />
        <textarea
          aria-label="Claude OAuth token"
          className="min-h-24 w-full resize-y rounded-md border border-input bg-background px-3 py-2 font-mono text-xs outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          onChange={(event) => setToken(event.target.value)}
          placeholder="Paste token — wrapped lines and whitespace are normalized"
          value={token}
        />
        {error ? <p className="text-sm text-destructive">{error}</p> : null}
        <Button
          disabled={!label.trim() || !token.trim() || addMutation.isPending}
          type="submit"
        >
          {addMutation.isPending ? "Verifying…" : "Verify and save"}
        </Button>
      </form>
    </section>
  );
}
