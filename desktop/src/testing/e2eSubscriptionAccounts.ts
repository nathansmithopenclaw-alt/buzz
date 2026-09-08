import type { RawManagedAgent } from "@/shared/api/tauri";
import type {
  ClaudeAccountChangeRequest,
  ClaudeAccountChangeStatus,
  SubscriptionAccount,
} from "@/shared/api/tauriSubscriptionAccounts";

let enabled = false;
let accounts: SubscriptionAccount[] = [];
let sequence = 0;
let defaults: Record<string, string | null> = {};
let group: ClaudeAccountChangeStatus | null = null;
const detached = new Set<string>();
export function resetMockSubscriptionAccounts() {
  enabled = false;
  accounts = [];
  sequence = 0;
  defaults = {};
  group = null;
  detached.clear();
}

/** Test bridge lifecycle: login starts pending, never fabricates real verification. */
export function mockSubscriptionAccountCommand(
  command: string,
  payload: Record<string, unknown>,
  agents: Pick<
    RawManagedAgent,
    | "name"
    | "status"
    | "subscription_account_inherit_default"
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
  const preview = (request: ClaudeAccountChangeRequest) => {
    const origin =
      request.kind === "agent"
        ? agents.find((agent) => agent.pubkey === request.pubkey)
        : null;
    const destination =
      request.kind === "remove" ? request.replacement : request.destination;
    const harness =
      accounts.find(
        (account) =>
          account.id ===
          (request.kind === "remove" ? request.account_id : destination),
      )?.harness ?? "claude";
    const defaultAccount = defaults[harness] ?? null;
    const source =
      request.kind === "remove"
        ? request.account_id
        : request.kind === "default"
          ? defaultAccount
          : origin?.subscription_account_id;
    const members = agents.filter((agent) =>
      request.kind === "default"
        ? agent.subscription_account_inherit_default &&
          agent.subscription_account_harness === harness
        : request.kind === "remove"
          ? agent.subscription_account_id === source
          : agent.pubkey === request.pubkey ||
            (request.all_on_current &&
              source &&
              agent.subscription_account_id === source),
    );
    const operation: ClaudeAccountChangeStatus = {
      id: "synthetic-group",
      harness: harness as "claude" | "codex" | "grok",
      phase: "applying",
      error: null,
      destination,
      change_default:
        request.kind === "default" ||
        (source != null &&
          source === defaultAccount &&
          (request.kind === "remove" || request.all_on_current)),
      removing: request.kind === "remove",
      members: members.map((agent) => ({
        pubkey: agent.pubkey,
        name: agent.name,
        running_count: agent.status === "running" ? 1 : 0,
        community_count: 1,
        done: false,
        error: null,
      })),
    };
    return {
      fingerprint: JSON.stringify([
        request,
        defaultAccount,
        members.map((agent) => [
          agent.pubkey,
          agent.subscription_account_id,
          agent.subscription_account_generation,
          agent.status,
        ]),
      ]),
      operation,
    };
  };
  switch (command) {
    case "get_claude_default_account":
      return defaults.claude ?? null;
    case "get_subscription_default_account":
      return defaults[String(payload.harness)] ?? null;
    case "get_claude_account_change_status":
      return structuredClone(group);
    case "preview_claude_account_change":
      return preview(payload.request as ClaudeAccountChangeRequest);
    case "apply_claude_account_change": {
      const request = payload.request as ClaudeAccountChangeRequest;
      const captured = preview(request);
      if (captured.fingerprint !== payload.fingerprint)
        throw new Error(
          "Account state changed. Review the affected agents again.",
        );
      group = captured.operation;
      for (const member of group.members) {
        const agent = agents.find((agent) => agent.pubkey === member.pubkey);
        if (!agent) throw new Error("Fixture agent unavailable");
        agent.subscription_account_id = group.destination;
        if (group.destination) detached.delete(agent.pubkey);
        else detached.add(agent.pubkey);
        agent.subscription_account_generation =
          (agent.subscription_account_generation ?? 0) + 1;
        if (request.kind === "default")
          agent.subscription_account_inherit_default = true;
        if (request.kind === "agent" && request.pubkey === agent.pubkey)
          agent.subscription_account_inherit_default = request.inherit_default;
        if (!group.destination) {
          agent.subscription_account_inherit_default = false;
          agent.status = "stopped";
          agent.running_subscription_account_id = null;
          agent.running_subscription_account_generation = null;
          agent.claude_account_id = null;
        }
        if (agent.status === "running") {
          agent.running_subscription_account_id = group.destination;
          agent.running_subscription_account_generation =
            agent.subscription_account_generation;
        }
        member.done = true;
      }
      if (group.change_default)
        defaults[group.harness ?? "claude"] = group.destination;
      if (request.kind === "remove")
        accounts = accounts.filter(
          (account) => account.id !== request.account_id,
        );
      group.phase = "complete";
      return structuredClone(group);
    }
    case "retry_claude_account_change":
      throw new Error("No interrupted account operation in this fixture");
    case "agent_account_switch_status":
      return detached.has(String(payload.pubkey))
        ? {
            id: "detached-fixture",
            account_id: "removed-fixture",
            phase: "paused",
            error:
              "Account detached. Choose a named account to resume retained work.",
          }
        : null;
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
          supported: true,
          reason: null,
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
      if (
        !token &&
        !["claude", "codex", "grok"].includes(String(payload.harness))
      )
        throw new Error("Unsupported subscription account harness");
      if (existing) {
        existing.state = "connecting";
        existing.attempt_id = `mock-attempt-${++sequence}`;
        return structuredClone(existing);
      }
      const row: SubscriptionAccount = {
        id: `mock-subscription-${++sequence}`,
        provider:
          token || payload.harness === "claude"
            ? "anthropic"
            : payload.harness === "grok"
              ? "xai"
              : "openai",
        harness: token
          ? "claude"
          : (payload.harness as "claude" | "codex" | "grok" | "grok"),
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
      agent.subscription_account_inherit_default = false;
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
