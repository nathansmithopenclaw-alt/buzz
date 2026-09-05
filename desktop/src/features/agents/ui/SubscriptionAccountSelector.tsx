import { subscriptionAccountExpiry } from "@/features/agents/subscriptionAccountExpiry";
import * as React from "react";
import {
  useIsMutating,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import {
  subscriptionAccountsQueryKey,
  useSubscriptionAccountsEnabledQuery,
  useSubscriptionAccountsQuery,
  useSubscriptionAccountClock,
} from "@/features/agents/subscriptionAccounts";
import {
  resumeAgentSubscriptionWork,
  switchAgentSubscriptionAccount,
} from "@/shared/api/tauriSubscriptionAccounts";
import type { ManagedAgent } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

/** Shared by the agent profile, Edit Agent and DM header; changes apply agent-wide. */
export function SubscriptionAccountSelector({
  agent,
  compact = false,
}: {
  agent: ManagedAgent;
  compact?: boolean;
}) {
  const accounts = useSubscriptionAccountsQuery();
  const now = useSubscriptionAccountClock();
  const enabled = useSubscriptionAccountsEnabledQuery();
  const client = useQueryClient();
  const switching =
    useIsMutating({
      mutationKey: ["switch-subscription-account", agent.pubkey],
    }) > 0;
  const working = useAgentWorking(agent.pubkey).working;
  const id = React.useId();
  const [confirmResume, setConfirmResume] = React.useState(false);
  const resume = useMutation({
    mutationFn: () => resumeAgentSubscriptionWork(agent.pubkey),
    onSuccess: async () => {
      setConfirmResume(false);
      await client.invalidateQueries({ queryKey: ["managed-agents"] });
    },
  });
  const selected = agent.subscriptionAccountId ?? null;
  const legacy = selected === null && agent.claudeAccountId !== null;
  const selectedAccount = accounts.data?.find(
    (account) => account.id === selected,
  );
  const expiry = subscriptionAccountExpiry(selectedAccount, now);
  const selectedValue = selected ?? (legacy ? "__legacy__" : "");
  const runningKnown = agent.runningSubscriptionAccountGeneration != null;
  const running = accounts.data?.find(
    (account) => account.id === agent.runningSubscriptionAccountId,
  );
  const mutation = useMutation({
    mutationKey: ["switch-subscription-account", agent.pubkey],
    mutationFn: (accountId: string | null) =>
      switchAgentSubscriptionAccount(agent.pubkey, accountId),
    onSettled: async () => {
      await Promise.all([
        client.invalidateQueries({ queryKey: ["managed-agents"] }),
        client.invalidateQueries({ queryKey: subscriptionAccountsQueryKey }),
      ]);
    },
  });
  const authHeld =
    agent.subscriptionAccountError === "subscription_auth_required";
  const error = mutation.isError
    ? String(mutation.error)
    : mutation.data?.failed_restart_count
      ? `Account selected; ${mutation.data.failed_restart_count} runtime restart failed. ${mutation.data.errors.join(" ")}`
      : authHeld
        ? null
        : agent.subscriptionAccountError;
  if (!agent.subscriptionAccountHarness && !selected && !legacy && !authHeld)
    return null;
  const unavailable =
    selected !== null &&
    (expiry?.expired ||
      selectedAccount?.state !== "ready" ||
      !selectedAccount.nickname ||
      selectedAccount.harness !== agent.subscriptionAccountHarness);
  const busy = working || switching || resume.isPending;
  const content = (
    <div
      className="space-y-1.5"
      data-testid={`subscription-account-selector-${agent.pubkey}`}
    >
      <label className="text-sm font-medium" htmlFor={id}>
        Subscription account
      </label>
      <select
        id={id}
        aria-describedby={`${id}-scope ${id}-status`}
        value={selectedValue}
        disabled={
          busy ||
          (!agent.subscriptionAccountHarness && !selected && !legacy) ||
          accounts.isPending ||
          accounts.isError ||
          enabled.isPending ||
          enabled.isError
        }
        className="w-full rounded-md border border-input bg-background px-2 py-1 text-sm"
        onChange={(event) => mutation.mutate(event.target.value || null)}
      >
        <option value="">Ambient — harness’s current login</option>
        {legacy ? (
          <option value="__legacy__" disabled>
            Legacy Claude account — re-login and rebind required
          </option>
        ) : null}
        {selected && !selectedAccount ? (
          <option value={selected} disabled>
            Selected account unavailable
          </option>
        ) : null}
        {accounts.data
          ?.filter(
            (account) =>
              account.id === selected ||
              (account.harness === agent.subscriptionAccountHarness &&
                account.state === "ready" &&
                account.nickname),
          )
          .map((account) => (
            <option
              key={account.id}
              value={account.id}
              disabled={
                subscriptionAccountExpiry(account, now)?.expired ||
                !enabled.data ||
                account.state !== "ready" ||
                !account.nickname ||
                account.harness !== agent.subscriptionAccountHarness
              }
            >
              {account.nickname ?? "Unnamed account"}
              {subscriptionAccountExpiry(account, now)
                ? ` — ${subscriptionAccountExpiry(account, now)?.label}`
                : ""}
              {account.state !== "ready"
                ? ` — ${account.state.replaceAll("_", " ")}`
                : ""}
            </option>
          ))}
      </select>
      <p id={`${id}-scope`} className="text-xs text-muted-foreground">
        Applies to this agent in every conversation.
      </p>
      <p
        id={`${id}-status`}
        role="status"
        className="text-xs text-muted-foreground"
      >
        {busy
          ? working
            ? "Finish the active turn before switching accounts."
            : "Applying account selection…"
          : runningKnown
            ? `Running: ${agent.runningSubscriptionAccountId ? (running?.nickname ?? "Unavailable account") : "Ambient — harness’s current login"}`
            : "Running account not confirmed."}
      </p>
      {expiry ? (
        <p className="text-xs" data-testid="selected-token-expiry">
          {expiry.label}
        </p>
      ) : null}
      {unavailable ? (
        <p className="text-xs text-destructive">
          Selected account unavailable.{" "}
          {selectedAccount?.kind === "token" ? "Renew" : "Reconnect"} it in
          Settings → AI; another account will not be chosen automatically.
        </p>
      ) : null}
      {legacy ? (
        <p className="text-xs text-muted-foreground">
          The legacy selection is preserved until you explicitly choose a
          verified account or Ambient.
        </p>
      ) : null}
      {enabled.data === false ? (
        <p className="text-xs text-muted-foreground">
          Experiment off. Existing selection stays in effect; selecting Ambient
          explicitly restores the harness’s current login.
        </p>
      ) : null}
      {accounts.isError || enabled.isError ? (
        <div role="alert" className="text-xs text-destructive">
          Could not load account settings.{" "}
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              void accounts.refetch();
              void enabled.refetch();
            }}
          >
            Retry
          </Button>
        </div>
      ) : null}
      {authHeld ? (
        <div className="space-y-2 text-xs">
          <p>
            {selectedAccount?.kind === "token"
              ? "Authentication paused this agent. Renew in Settings → AI, then resume retained work."
              : "Authentication paused this agent. Stop the agent, reconnect in Settings → AI, then start it and resume retained work."}
          </p>
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => setConfirmResume(true)}
          >
            Resume retained work
          </Button>
          {confirmResume ? (
            <div>
              <p>
                Earlier partial work may already have run. Resume the original
                requests now?
              </p>
              <Button size="sm" disabled={busy} onClick={() => resume.mutate()}>
                Resume now
              </Button>{" "}
              <Button
                size="sm"
                variant="ghost"
                disabled={busy}
                onClick={() => setConfirmResume(false)}
              >
                Keep paused
              </Button>
            </div>
          ) : null}
        </div>
      ) : null}
      {resume.isError ? (
        <p role="alert" className="text-xs text-destructive">
          {String(resume.error)}
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
  if (!compact) return content;
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          size="sm"
          variant="outline"
          className="h-7 max-w-48 truncate rounded-full text-xs"
          aria-label="Change subscription account"
          title="Account selection applies to this agent in every conversation"
        >
          {error || unavailable
            ? "Account needs attention"
            : (selectedAccount?.nickname ??
              (legacy ? "Legacy Claude account" : "Ambient account"))}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80">
        {content}
      </PopoverContent>
    </Popover>
  );
}
