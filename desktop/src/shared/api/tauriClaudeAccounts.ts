import { invokeTauri } from "@/shared/api/tauri";

export const CLAUDE_CREDENTIALS_FILE_ACCOUNT_ID = "__credentials_file__";

export type ClaudeUsageWindow = {
  utilization: number | null;
  resetsAt: string | null;
};

export type ClaudeModelUsage = ClaudeUsageWindow & { model: string };

export type ClaudeAccount = {
  id: string;
  label: string;
  fingerprint: string;
  length: number;
  lastVerified: string | null;
  planType: string | null;
  fiveHour: ClaudeUsageWindow | null;
  weekly: ClaudeUsageWindow | null;
  modelScoped: ClaudeModelUsage[];
  usageStatus: "available" | "scope_insufficient" | "stale" | "unavailable";
  usageHttpStatus: number | null;
  usageError: string | null;
  usageCheckedAt: string | null;
};

type RawWindow = { utilization: number | null; resets_at: string | null };
type RawAccount = {
  id: string;
  label: string;
  fingerprint: string;
  length: number;
  last_verified: string | null;
  plan_type: string | null;
  five_hour: RawWindow | null;
  weekly: RawWindow | null;
  model_scoped: Array<RawWindow & { model: string }>;
  usage_status: ClaudeAccount["usageStatus"];
  usage_http_status: number | null;
  usage_error: string | null;
  usage_checked_at: string | null;
};

function windowFromRaw(value: RawWindow | null): ClaudeUsageWindow | null {
  return value
    ? { utilization: value.utilization, resetsAt: value.resets_at }
    : null;
}

function fromRaw(value: RawAccount): ClaudeAccount {
  return {
    id: value.id,
    label: value.label,
    fingerprint: value.fingerprint,
    length: value.length,
    lastVerified: value.last_verified,
    planType: value.plan_type,
    fiveHour: windowFromRaw(value.five_hour),
    weekly: windowFromRaw(value.weekly),
    modelScoped: value.model_scoped.map((entry) => ({
      model: entry.model,
      utilization: entry.utilization,
      resetsAt: entry.resets_at,
    })),
    usageStatus: value.usage_status,
    usageHttpStatus: value.usage_http_status,
    usageError: value.usage_error,
    usageCheckedAt: value.usage_checked_at,
  };
}

export async function listClaudeAccounts(): Promise<ClaudeAccount[]> {
  return (await invokeTauri<RawAccount[]>("list_claude_accounts")).map(fromRaw);
}

export async function addClaudeAccount(
  label: string,
  token: string,
): Promise<ClaudeAccount> {
  return fromRaw(
    await invokeTauri<RawAccount>("add_claude_account", { label, token }),
  );
}

export async function deleteClaudeAccount(id: string): Promise<void> {
  await invokeTauri("delete_claude_account", { id });
}

export type ClaudeAccountSwitchResult = {
  account_id: string | null;
  fingerprint: string | null;
  restarted_count: number;
  failed_restart_count: number;
  verified_runtime_count: number;
  running_pids: number[];
};

export async function switchAgentClaudeAccount(
  pubkey: string,
  accountId: string | null,
  deferredAfterTurn = false,
): Promise<ClaudeAccountSwitchResult> {
  return invokeTauri("switch_agent_claude_account", {
    pubkey,
    accountId,
    deferredAfterTurn,
  });
}

export async function noteClaudeAccountSwitchQueued(
  pubkey: string,
): Promise<void> {
  await invokeTauri("note_claude_account_switch_queued", { pubkey });
}
