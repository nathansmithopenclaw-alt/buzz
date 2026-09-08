import { subscriptionAccountExpiry } from "@/features/agents/subscriptionAccountExpiry";
import * as React from "react";
import { ChevronDown } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  useIsMutating,
  useMutation,
  useQueryClient,
  useQuery,
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
  agentAccountSwitchStatus,
  switchAgentSubscriptionAccount,
} from "@/shared/api/tauriSubscriptionAccounts";
import type { ManagedAgent } from "@/shared/api/types";
import {
  useProviderDefaultAccount,
  useClaudeAccountChange,
} from "../accountChanges";
import { ClaudeAccountChangeDialog } from "./ClaudeAccountChangeDialog";
import { ClaudeAccountChangeProgress } from "./ClaudeAccountChangeProgress";
import type { ClaudeAccountChangeRequest } from "@/shared/api/tauriSubscriptionAccounts";
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
  const defaultAccount = useProviderDefaultAccount(
    agent.subscriptionAccountHarness ?? "claude",
  );
  const group = useClaudeAccountChange();
  const operation = useQuery({
    queryKey: ["account-switch", agent.pubkey],
    queryFn: () => agentAccountSwitchStatus(agent.pubkey),
    refetchInterval: 750,
    enabled:
      agent.subscriptionAccountHarness === "claude" ||
      agent.subscriptionAccountHarness === "codex" ||
      agent.subscriptionAccountHarness === "grok",
  });
  const operationRevision = operation.data
    ? JSON.stringify([
        operation.data.id,
        operation.data.phase,
        operation.data.error,
      ])
    : null;
  React.useEffect(() => {
    // Startup recovery has no picker mutation to invalidate the cached stopped
    // agent. Refresh once per durable transition, including the initial read.
    if (operationRevision)
      void client.invalidateQueries({ queryKey: ["managed-agents"] });
  }, [client, operationRevision]);
  const [destination, setDestination] = React.useState<
    ClaudeAccountChangeRequest | undefined
  >();
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
  const accountRequired = selected === null && Boolean(operation.data);
  const legacy =
    agent.subscriptionAccountHarness === "claude" &&
    selected === null &&
    !accountRequired &&
    agent.claudeAccountId !== null;
  const selectedAccount = accounts.data?.find(
    (account) => account.id === selected,
  );
  const expiry = subscriptionAccountExpiry(selectedAccount, now);
  const selectedValue = accountRequired
    ? "__required__"
    : agent.subscriptionAccountInheritDefault
      ? "__default__"
      : (selected ?? (legacy ? "__legacy__" : ""));
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
        client.invalidateQueries({
          queryKey: ["account-switch", agent.pubkey],
        }),
      ]);
    },
  });
  const authHeld =
    agent.subscriptionAccountError === "subscription_auth_required";
  const error =
    operation.data?.error ??
    (mutation.isError
      ? String(mutation.error)
      : mutation.data?.failed_restart_count
        ? `Account selected; ${mutation.data.failed_restart_count} runtime restart failed. ${mutation.data.errors.join(" ")}`
        : authHeld
          ? null
          : agent.subscriptionAccountError);
  if (!agent.subscriptionAccountHarness && !selected && !legacy && !authHeld)
    return null;
  const unavailable =
    selected !== null &&
    (expiry?.expired ||
      selectedAccount?.state !== "ready" ||
      !selectedAccount.nickname ||
      selectedAccount.harness !== agent.subscriptionAccountHarness);
  const activeSwitch = Boolean(
    operation.data &&
      operation.data.phase !== "complete" &&
      !operation.data.error,
  );
  const canPause =
    ["claude", "codex", "grok"].includes(
      agent.subscriptionAccountHarness ?? "",
    ) &&
    accounts.data?.some(
      (account) =>
        account.harness === agent.subscriptionAccountHarness &&
        (account.harness !== "claude" || account.kind === "token") &&
        account.state === "ready",
    );
  const groupPending = Boolean(group.data && group.data.phase !== "complete");
  const busy =
    switching ||
    resume.isPending ||
    activeSwitch ||
    groupPending ||
    (working && !canPause);
  const selectAccount = (value: string) => {
    const inheritDefault = value === "__default__";
    const target = inheritDefault ? defaultAccount.data : value;
    if (!target) return;
    if (
      target === selected &&
      inheritDefault === Boolean(agent.subscriptionAccountInheritDefault) &&
      !authHeld &&
      !error
    )
      return;
    const account = accounts.data?.find((item) => item.id === target);
    if (
      account?.harness === "codex" ||
      account?.harness === "grok" ||
      (account?.harness === "claude" && account.kind === "token")
    )
      setDestination({
        kind: "agent",
        pubkey: agent.pubkey,
        destination: target,
        inherit_default: inheritDefault,
        all_on_current: false,
      });
    else mutation.mutate(value === "__ambient__" ? null : value);
  };
  const content = (
    <div
      className="space-y-1.5"
      data-testid={`subscription-account-selector-${agent.pubkey}`}
    >
      <label className="text-sm font-medium" htmlFor={id}>
        Subscription account
      </label>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            id={id}
            variant="outline"
            aria-describedby={`${id}-scope ${id}-status`}
            disabled={
              busy ||
              (!agent.subscriptionAccountHarness && !selected && !legacy) ||
              accounts.isPending ||
              accounts.isError ||
              enabled.isPending ||
              enabled.isError
            }
            className="w-full justify-between gap-2 font-normal"
          >
            <span className="truncate">
              {agent.subscriptionAccountInheritDefault ? "Default · " : ""}
              {accountRequired
                ? "Account required — choose a named account"
                : (selectedAccount?.nickname ??
                  (selected
                    ? "Selected account unavailable"
                    : legacy
                      ? "Legacy Claude account"
                      : "Ambient — harness’s current login"))}
            </span>
            <ChevronDown className="size-4 shrink-0 text-muted-foreground" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="start"
          className="min-w-[var(--radix-dropdown-menu-trigger-width)]"
        >
          <DropdownMenuRadioGroup
            value={selectedValue || "__ambient__"}
            onValueChange={selectAccount}
          >
            {accountRequired ? (
              <DropdownMenuRadioItem value="__required__" disabled>
                Account required — choose a named account
              </DropdownMenuRadioItem>
            ) : null}
            {canPause && defaultAccount.data ? (
              <DropdownMenuRadioItem
                value="__default__"
                disabled={!enabled.data || defaultAccount.isError}
              >
                Use default ·{" "}
                {accounts.data?.find(
                  (account) => account.id === defaultAccount.data,
                )?.nickname ?? "Unavailable account"}
              </DropdownMenuRadioItem>
            ) : null}
            <DropdownMenuRadioItem
              value="__ambient__"
              disabled={working || Boolean(operation.data)}
            >
              Ambient — harness’s current login
            </DropdownMenuRadioItem>
            {legacy ? (
              <DropdownMenuRadioItem value="__legacy__" disabled>
                Legacy Claude account — re-login and rebind required
              </DropdownMenuRadioItem>
            ) : null}
            {selected && !selectedAccount ? (
              <DropdownMenuRadioItem value={selected} disabled>
                Selected account unavailable
              </DropdownMenuRadioItem>
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
                <DropdownMenuRadioItem
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
                </DropdownMenuRadioItem>
              ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      <p id={`${id}-scope`} className="text-xs text-muted-foreground">
        Applies to this agent in every conversation.
      </p>
      <p
        id={`${id}-status`}
        role="status"
        className="text-xs text-muted-foreground"
      >
        {working && !canPause
          ? "Finish the active turn before switching accounts."
          : switching || activeSwitch
            ? operation.data?.phase === "restarting"
              ? "Starting the selected account and resuming saved work…"
              : "Pausing this agent and saving unfinished work…"
            : runningKnown
              ? `Running: ${agent.runningSubscriptionAccountId ? (running?.nickname ?? "Unavailable account") : "Ambient — harness’s current login"}`
              : agent.status === "running"
                ? "Waiting for runtime account confirmation."
                : "Agent stopped. This selection will apply when it starts."}
      </p>
      {destination ? (
        <ClaudeAccountChangeDialog
          request={destination}
          allowAll={Boolean(selected)}
          onClose={() => setDestination(undefined)}
        />
      ) : null}
      <ClaudeAccountChangeProgress pubkey={agent.pubkey} />
      {(error || authHeld) &&
      !groupPending &&
      (selectedAccount?.kind === "token" ||
        selectedAccount?.harness === "codex" ||
        selectedAccount?.harness === "grok") ? (
        <div className="space-y-1.5 text-xs">
          <p>
            Work is retained. Retry this account or choose another account
            above.
          </p>
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() =>
              mutation.mutate(operation.data?.account_id ?? selected)
            }
          >
            Retry account switch
          </Button>
        </div>
      ) : null}
      {expiry ? (
        <p className="text-xs" data-testid="selected-token-expiry">
          {expiry.label}
        </p>
      ) : null}
      {unavailable ? (
        <p className="text-xs text-destructive">
          Selected account unavailable.{" "}
          {selectedAccount?.kind === "token" ? "Renew" : "Reconnect"} in
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
      {authHeld && selectedAccount?.kind !== "token" ? (
        <div className="space-y-2 text-xs">
          <p>
            Authentication paused this agent. Stop the agent, reconnect in
            Settings → AI, then start it and resume retained work.
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
