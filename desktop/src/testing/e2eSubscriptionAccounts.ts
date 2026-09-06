import type { RawManagedAgent } from "@/shared/api/tauri";
import type { SubscriptionAccount } from "@/shared/api/tauriSubscriptionAccounts";

let enabled = false;
let accounts: SubscriptionAccount[] = [];
let sequence = 0;
export function resetMockSubscriptionAccounts() {
  enabled = false;
  accounts = [];
  sequence = 0;
}

/** Test bridge lifecycle: login starts pending, never fabricates real verification. */
export function mockSubscriptionAccountCommand(
  command: string,
  payload: Record<string, unknown>,
  agents: Pick<
    RawManagedAgent,
    | "pubkey"
    | "subscription_account_id"
    | "subscription_account_generation"
    | "running_subscription_account_id"
    | "running_subscription_account_generation"
    | "subscription_account_harness"
    | "claude_account_id"
  >[],
): unknown {
  const id = String(payload.id ?? "");
  const account = accounts.find((row) => row.id === id);
  switch (command) {
    case "agent_account_switch_status":
      return null;
    case "get_subscription_accounts_enabled":
      return enabled;
    case "set_subscription_accounts_enabled":
      enabled = Boolean(payload.enabled);
      return;
    case "subscription_account_capabilities":
      return [
        {
          provider: "anthropic",
          harness: "claude",
          token_supported: true,
          supported: false,
          reason: "Claude accounts use long-lived tokens",
        },
        { provider: "openai", harness: "codex", supported: true, reason: null },
        {
          provider: "xai",
          harness: "grok",
          supported: false,
          reason:
            "Grok subscription status, logout and configuration preservation are not yet validated",
        },
      ];
    case "list_subscription_accounts":
      return structuredClone(accounts);
    case "begin_subscription_account_token":
    case "begin_subscription_account_login": {
      const token = command === "begin_subscription_account_token";
      if (accounts.some((row) => row.state === "connecting"))
        throw new Error("A login is already in progress");
      const existing = accounts.find((row) => row.id === payload.accountId);
      if (!existing && !enabled)
        throw new Error("Enable Subscription Accounts first");
      if (!token && payload.harness !== "claude" && payload.harness !== "codex")
        throw new Error("Unsupported subscription account harness");
      if (existing) {
        existing.state = "connecting";
        existing.attempt_id = `mock-attempt-${++sequence}`;
        return structuredClone(existing);
      }
      const row: SubscriptionAccount = {
        id: `mock-subscription-${++sequence}`,
        provider:
          token || payload.harness === "claude" ? "anthropic" : "openai",
        harness: token ? "claude" : (payload.harness as "claude" | "codex"),
        kind: token ? "token" : "login",
        minted_at: null,
        expires_at: null,
        nickname: null,
        state: "connecting",
        last_checked_at: null,
        last_error: null,
        attempt_id: `mock-attempt-${sequence}`,
      };
      accounts.push(row);
      return structuredClone(row);
    }
    case "cancel_subscription_account_login":
      if (!account) throw new Error("Account unavailable");
      account.state = "needs_login";
      account.attempt_id = null;
      account.last_error = "Login cancelled";
      return structuredClone(account);
    case "rename_subscription_account":
      if (account?.state !== "ready")
        throw new Error("Verify the login before naming this account");
      account.nickname = String(payload.nickname).trim();
      return structuredClone(account);
    case "remove_subscription_account":
      if (!account) throw new Error("Account unavailable");
      if (
        agents.some(
          (agent) =>
            agent.subscription_account_id === id ||
            (agent.running_subscription_account_generation != null &&
              agent.running_subscription_account_id === id),
        )
      )
        throw new Error("Account is selected or running");
      accounts = accounts.filter((row) => row.id !== id);
      return;
    case "resume_agent_subscription_work":
      throw new Error("No authentication-held work in this fixture");
    case "switch_agent_subscription_account": {
      const agent = agents.find((row) => row.pubkey === payload.pubkey);
      if (!agent) throw new Error("Agent unavailable");
      const selectedId =
        typeof payload.accountId === "string" ? payload.accountId : null;
      const selected = accounts.find((row) => row.id === selectedId);
      if (
        selectedId &&
        (!enabled ||
          !selected ||
          selected.state !== "ready" ||
          selected.harness !== agent.subscription_account_harness)
      )
        throw new Error("Subscription account unavailable");
      agent.subscription_account_id = selectedId;
      agent.subscription_account_generation =
        (agent.subscription_account_generation ?? 0) + 1;
      agent.claude_account_id = null;
      return {
        account_id: selectedId,
        generation: agent.subscription_account_generation,
        restarted_count: 0,
        failed_restart_count: 0,
        errors: [],
      };
    }
    default:
      throw new Error(`Unsupported mock subscription command: ${command}`);
  }
}

/** Explicit synthetic provider completion, callable only inside the E2E bridge. */
export function completeMockSubscriptionToken(
  accountId: string,
  mintedAt: number,
) {
  const account = accounts.find((row) => row.id === accountId);
  if (account?.kind !== "token" || account.state !== "connecting")
    throw new Error("No pending synthetic token setup");
  Object.assign(account, {
    state: "ready",
    minted_at: mintedAt,
    expires_at: mintedAt + 365 * 86400,
    attempt_id: null,
    last_error: null,
  });
}
