import { invokeTauri } from "./tauri";

/** Local metadata only; credentials and authorization URLs never cross IPC. */
export type SubscriptionAccount = {
  id: string;
  kind?: "login" | "token";
  minted_at?: number | null;
  expires_at?: number | null;
  provider: "anthropic" | "openai" | "xai";
  harness: "claude" | "codex" | "grok";
  nickname: string | null;
  state:
    | "connecting"
    | "ready"
    | "needs_login"
    | "removal_pending"
    | "unavailable";
  last_checked_at: string | null;
  last_error: string | null;
  attempt_id: string | null;
};

export type SubscriptionAccountCapability = {
  provider: SubscriptionAccount["provider"];
  harness: SubscriptionAccount["harness"];
  supported: boolean;
  token_supported?: boolean;
  reason: string | null;
};

export const listSubscriptionAccounts = () =>
  invokeTauri<SubscriptionAccount[]>("list_subscription_accounts");
export const subscriptionAccountCapabilities = () =>
  invokeTauri<SubscriptionAccountCapability[]>(
    "subscription_account_capabilities",
  );
export const beginSubscriptionAccountLogin = (
  harness: string,
  accountId: string | null = null,
) =>
  invokeTauri<SubscriptionAccount>("begin_subscription_account_login", {
    harness,
    accountId,
  });
/** Mint or renew locally; only lifecycle metadata is returned to the UI. */
export const beginSubscriptionAccountToken = (
  accountId: string | null = null,
) =>
  invokeTauri<SubscriptionAccount>("begin_subscription_account_token", {
    accountId,
  });
export const cancelSubscriptionAccountLogin = (id: string) =>
  invokeTauri<SubscriptionAccount>("cancel_subscription_account_login", { id });
export const renameSubscriptionAccount = (id: string, nickname: string) =>
  invokeTauri<SubscriptionAccount>("rename_subscription_account", {
    id,
    nickname,
  });
export const removeSubscriptionAccount = (id: string) =>
  invokeTauri<void>("remove_subscription_account", { id });
export const getSubscriptionAccountsEnabled = () =>
  invokeTauri<boolean>("get_subscription_accounts_enabled");
export const setSubscriptionAccountsEnabled = (enabled: boolean) =>
  invokeTauri<void>("set_subscription_accounts_enabled", { enabled });

export type SubscriptionAccountSwitchResult = {
  account_id: string | null;
  generation: number;
  restarted_count: number;
  failed_restart_count: number;
  errors: string[];
};
export const switchAgentSubscriptionAccount = (
  pubkey: string,
  accountId: string | null,
) =>
  invokeTauri<SubscriptionAccountSwitchResult>(
    "switch_agent_subscription_account",
    { pubkey, accountId },
  );

export const resumeAgentSubscriptionWork = (pubkey: string) =>
  invokeTauri<void>("resume_agent_subscription_work", { pubkey });

export type AccountSwitchStatus = {
  id: string;
  account_id: string;
  phase: "pausing" | "paused" | "restarting" | "complete";
  error: string | null;
};
export const agentAccountSwitchStatus = (pubkey: string) =>
  invokeTauri<AccountSwitchStatus | null>("agent_account_switch_status", {
    pubkey,
  });

export type ClaudeAccountChangeRequest =
  | {
      kind: "agent";
      pubkey: string;
      destination: string;
      inherit_default: boolean;
      all_on_current: boolean;
    }
  | { kind: "default"; destination: string }
  | { kind: "remove"; account_id: string; replacement: string | null };
export type ClaudeAccountChangeStatus = {
  harness?: "claude" | "codex";
  id: string;
  phase: "applying" | "paused" | "removing" | "complete";
  error: string | null;
  destination: string | null;
  change_default: boolean;
  removing: boolean;
  members: Array<{
    pubkey: string;
    name: string;
    running_count: number;
    community_count: number;
    done: boolean;
    error: string | null;
  }>;
};
export type ClaudeAccountChangePreview = {
  fingerprint: string;
  operation: ClaudeAccountChangeStatus;
};
export const getSubscriptionDefaultAccount = (harness: string) =>
  invokeTauri<string | null>("get_subscription_default_account", { harness });

export const getClaudeDefaultAccount = () =>
  invokeTauri<string | null>("get_claude_default_account");
export const previewClaudeAccountChange = (
  request: ClaudeAccountChangeRequest,
) =>
  invokeTauri<ClaudeAccountChangePreview>("preview_claude_account_change", {
    request,
  });
export const applyClaudeAccountChange = (
  request: ClaudeAccountChangeRequest,
  fingerprint: string,
) =>
  invokeTauri<ClaudeAccountChangeStatus>("apply_claude_account_change", {
    request,
    fingerprint,
  });
export const retryClaudeAccountChange = (
  id: string,
  destination: string | null = null,
) =>
  invokeTauri<ClaudeAccountChangeStatus>("retry_claude_account_change", {
    id,
    destination,
  });
export const getClaudeAccountChangeStatus = () =>
  invokeTauri<ClaudeAccountChangeStatus | null>(
    "get_claude_account_change_status",
  );
