import * as React from "react";
import { AlertTriangle, Check, ChevronDown, Clock3 } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";

import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import {
  claudeAccountsQueryKey,
  useClaudeAccountsQuery,
} from "@/features/agents/claudeAccounts";
import { managedAgentsQueryKey } from "@/features/agents/hooks";
import {
  CLAUDE_CREDENTIALS_FILE_ACCOUNT_ID,
  switchAgentClaudeAccount,
} from "@/shared/api/tauriClaudeAccounts";
import {
  clearClaudeAccountSwitch,
  queueClaudeAccountSwitch,
  subscribeClaudeAccountSwitch,
  type ClaudeAccountSwitchState,
} from "@/features/agents/claudeAccountSwitchQueue";
import type { ManagedAgent } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { cn } from "@/shared/lib/cn";

function compactReset(value: string | null | undefined) {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return `resets ${date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`;
}

export function ClaudeAccountPill({
  agent,
  compact = false,
}: {
  agent: ManagedAgent;
  compact?: boolean;
}) {
  const accountsQuery = useClaudeAccountsQuery();
  const queryClient = useQueryClient();
  const working = useAgentWorking(agent.pubkey).working;
  const [queuedSwitch, setQueuedSwitch] =
    React.useState<ClaudeAccountSwitchState | null>(null);
  const [switching, setSwitching] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const current =
    accountsQuery.data?.find(
      (account) => account.id === agent.claudeAccountId,
    ) ?? null;

  const apply = React.useCallback(
    async (accountId: string | null, deferredAfterTurn = false) => {
      setSwitching(true);
      setError(null);
      try {
        const result = await switchAgentClaudeAccount(
          agent.pubkey,
          accountId,
          deferredAfterTurn,
        );
        if (result.failed_restart_count > 0) {
          setError(`${result.failed_restart_count} runtime restart failed`);
        } else if (result.restarted_count !== result.verified_runtime_count) {
          setError(
            "Account saved, but the restarted process could not be verified",
          );
        }
        await queryClient.invalidateQueries({
          queryKey: managedAgentsQueryKey,
        });
        await queryClient.invalidateQueries({
          queryKey: claudeAccountsQueryKey,
        });
      } catch (cause) {
        setError(String(cause));
      } finally {
        setSwitching(false);
      }
    },
    [agent.pubkey, queryClient],
  );

  React.useEffect(
    () => subscribeClaudeAccountSwitch(agent.pubkey, setQueuedSwitch),
    [agent.pubkey],
  );

  React.useEffect(() => {
    if (queuedSwitch?.status === "failed") {
      setError(queuedSwitch.error);
      clearClaudeAccountSwitch(agent.pubkey);
    } else if (queuedSwitch?.status === "complete") {
      const result = queuedSwitch.result;
      if (result.failed_restart_count > 0) {
        setError(`${result.failed_restart_count} runtime restart failed`);
      } else if (result.restarted_count !== result.verified_runtime_count) {
        setError(
          "Account saved, but the restarted process could not be verified",
        );
      }
      void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
      void queryClient.invalidateQueries({ queryKey: claudeAccountsQueryKey });
      clearClaudeAccountSwitch(agent.pubkey);
    }
  }, [agent.pubkey, queryClient, queuedSwitch]);

  const choose = (accountId: string | null) => {
    if (accountId === agent.claudeAccountId) return;
    if (working) {
      queueClaudeAccountSwitch(agent.pubkey, accountId);
    } else {
      void apply(accountId);
    }
  };

  const usage = current?.fiveHour?.utilization;
  const label = current
    ? `${current.label}${usage == null ? "" : ` · ${Math.round(usage)}%`}`
    : "Credentials file fallback";
  const title =
    queuedSwitch?.status === "queued"
      ? "Account switch queued — the active turn will finish before restart"
      : agent.claudeAccountSource === "credentials_file"
        ? "Falling back to credentials file (no env token)"
        : agent.claudeAccountShadowed
          ? "Definition account is shadowed by a per-agent override"
          : agent.claudeAccountSource === "instance"
            ? "Pinned by a per-agent selection"
            : agent.claudeAccountSource === "global"
              ? "Inherited from global agent defaults"
              : "Inherited from agent definition";

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          className={cn(
            "h-7 max-w-64 gap-1.5 rounded-full px-2.5 text-xs",
            error && "border-destructive/60",
          )}
          data-testid={`claude-account-pill-${agent.pubkey}`}
          disabled={switching}
          size="sm"
          title={error ?? title}
          variant="outline"
        >
          {queuedSwitch?.status === "queued" ? (
            <Clock3 className="h-3.5 w-3.5 text-amber-500" />
          ) : agent.claudeAccountSource === "credentials_file" ||
            agent.claudeAccountShadowed ? (
            <AlertTriangle className="h-3.5 w-3.5 text-amber-500" />
          ) : null}
          <span className="truncate">
            {queuedSwitch?.status === "queued"
              ? "Switch queued"
              : switching || queuedSwitch?.status === "applying"
                ? "Applying…"
                : label}
          </span>
          {!compact ? (
            <span className="hidden text-muted-foreground xl:inline">
              {compactReset(current?.fiveHour?.resetsAt)}
            </span>
          ) : null}
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-72">
        <DropdownMenuLabel>Claude account</DropdownMenuLabel>
        <DropdownMenuItem
          onSelect={() => choose(CLAUDE_CREDENTIALS_FILE_ACCOUNT_ID)}
        >
          <span className="flex-1">Credentials file fallback</span>
          {agent.claudeAccountId === null ? (
            <Check className="h-4 w-4" />
          ) : null}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        {accountsQuery.data?.map((account) => (
          <DropdownMenuItem
            key={account.id}
            onSelect={() => choose(account.id)}
          >
            <span className="flex-1">
              <span className="block">{account.label}</span>
              <span className="block text-xs text-muted-foreground">
                {account.usageStatus === "available"
                  ? `${Math.round(account.fiveHour?.utilization ?? 0)}% · ${compactReset(account.fiveHour?.resetsAt) ?? "reset unavailable"}`
                  : account.usageStatus === "scope_insufficient"
                    ? "Meter needs interactive-login token"
                    : account.usageStatus === "stale"
                      ? "Usage temporarily rate limited"
                      : "Usage unavailable"}
              </span>
            </span>
            {agent.claudeAccountId === account.id ? (
              <Check className="h-4 w-4" />
            ) : null}
          </DropdownMenuItem>
        ))}
        {accountsQuery.data?.length === 0 ? (
          <DropdownMenuItem disabled>
            No saved accounts — add one in Settings → AI
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
