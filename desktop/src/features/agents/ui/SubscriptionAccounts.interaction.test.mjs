import assert from "node:assert/strict";
import { after, afterEach, beforeEach, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Element: dom.window.Element,
  HTMLIFrameElement: dom.window.HTMLIFrameElement,
  Node: dom.window.Node,
  NodeFilter: dom.window.NodeFilter,
  CustomEvent: dom.window.CustomEvent,
  Event: dom.window.Event,
  HTMLInputElement: dom.window.HTMLInputElement,
  getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
  requestAnimationFrame: (callback) => setTimeout(callback, 0),
  cancelAnimationFrame: clearTimeout,
  ResizeObserver: class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
  MutationObserver: dom.window.MutationObserver,
  localStorage: dom.window.localStorage,
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", {
  value: dom.window.navigator,
  configurable: true,
});
const calls = [];
let accounts, enabled, failSwitch, failRemove, failCancel, operationStatus;
const capabilities = [
  {
    provider: "anthropic",
    harness: "claude",
    supported: false,
    token_supported: true,
    reason: null,
  },
  { provider: "openai", harness: "codex", supported: true, reason: null },
  {
    provider: "xai",
    harness: "grok",
    supported: false,
    reason: "Adapter acceptance pending",
  },
];
globalThis.__TAURI_INTERNALS__ = dom.window.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    calls.push([command, args]);
    switch (command) {
      case "list_subscription_accounts":
        return structuredClone(accounts);
      case "subscription_account_capabilities":
        return capabilities;
      case "get_subscription_accounts_enabled":
        return enabled;
      case "set_subscription_accounts_enabled":
        enabled = args.enabled;
        return;
      case "begin_subscription_account_token":
      case "begin_subscription_account_login": {
        const existing = accounts.find(
          (account) => account.id === args.accountId,
        );
        if (existing)
          Object.assign(existing, {
            state: "connecting",
            attempt_id: "attempt-new",
          });
        else
          accounts.push(
            account({
              nickname: null,
              kind:
                command === "begin_subscription_account_token"
                  ? "token"
                  : "login",
              harness:
                command === "begin_subscription_account_token"
                  ? "claude"
                  : "codex",
              state: "connecting",
              attempt_id: "attempt-new",
            }),
          );
        return structuredClone(existing ?? accounts.at(-1));
      }
      case "cancel_subscription_account_login": {
        if (failCancel)
          throw new Error("Process cleanup failed; retry cancellation");
        const row = accounts.find((item) => item.id === args.id);
        Object.assign(row, {
          state: "needs_login",
          attempt_id: null,
          last_error: "Login cancelled",
        });
        return structuredClone(row);
      }
      case "rename_subscription_account": {
        const row = accounts.find((item) => item.id === args.id);
        row.nickname = args.nickname;
        return structuredClone(row);
      }
      case "remove_subscription_account":
        if (failRemove)
          throw new Error("Account is still selected by an agent");
        accounts = accounts.filter((item) => item.id !== args.id);
        return;
      case "agent_account_switch_status":
        return operationStatus ?? null;
      case "resume_agent_subscription_work":
        return;
      case "switch_agent_subscription_account":
        return {
          account_id: args.accountId,
          generation: 2,
          restarted_count: 0,
          failed_restart_count: failSwitch ? 1 : 0,
          errors: failSwitch ? ["Restart could not start"] : [],
        };
      default:
        throw new Error(`Unexpected IPC ${command}`);
    }
  },
  transformCallback: () => 1,
};
const React = await import("react");
const { render, cleanup, fireEvent, waitFor } = await import(
  "@testing-library/react"
);
const { focusManager, QueryClient, QueryClientProvider, useQuery } =
  await import("@tanstack/react-query");
const { SubscriptionAccountSelector } = await import(
  "./SubscriptionAccountSelector.tsx"
);
const { SubscriptionAccountsSettingsCard } = await import(
  "../../settings/ui/SubscriptionAccountsSettingsCard.tsx"
);
const { SubscriptionAccountsExperimentRow } = await import(
  "../../settings/ui/SubscriptionAccountsExperimentRow.tsx"
);
const { reportChannelBotTyping } = await import("../agentWorkingSignal.ts");
let client;
const pubkey = "a".repeat(64);
function account(overrides = {}) {
  return {
    id: "account-a",
    provider: "openai",
    harness: "codex",
    nickname: "Work",
    state: "ready",
    last_checked_at: "2026-09-04T12:00:00Z",
    last_error: null,
    attempt_id: null,
    ...overrides,
  };
}
function agent(overrides = {}) {
  return {
    pubkey,
    claudeAccountId: null,
    subscriptionAccountHarness: "codex",
    subscriptionAccountId: null,
    subscriptionAccountGeneration: 0,
    runningSubscriptionAccountId: null,
    runningSubscriptionAccountGeneration: null,
    ...overrides,
  };
}
function mount(...children) {
  return render(
    React.createElement(QueryClientProvider, { client }, ...children),
  );
}
async function openAccountMenu(view, picker) {
  picker ??= await view.findByRole("button", { name: "Subscription account" });
  await waitFor(() => assert.equal(picker.disabled, false));
  fireEvent.keyDown(picker, { key: "ArrowDown", code: "ArrowDown" });
  await view.findByRole("menu");
}
function closeAccountMenu(view) {
  fireEvent.keyDown(view.getByRole("menu"), { key: "Escape", code: "Escape" });
}
async function chooseAccount(view, name) {
  await openAccountMenu(view);
  fireEvent.click(await view.findByRole("menuitemradio", { name }));
}
async function refresh() {
  await React.act(async () =>
    client.invalidateQueries({ queryKey: ["subscription-accounts"] }),
  );
}
beforeEach(() => {
  operationStatus = null;
  accounts = [];
  enabled = true;
  failSwitch = false;
  failRemove = false;
  failCancel = false;
  calls.length = 0;
  client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity, refetchOnWindowFocus: false },
      mutations: { retry: false, gcTime: Infinity },
    },
  });
});
afterEach(() => {
  cleanup();
  client.clear();
  focusManager.setFocused(undefined);
  reportChannelBotTyping("other-channel", []);
});
after(() => dom.window.close());

test("startup switch recovery refreshes a cached stopped agent without a picker mutation", async () => {
  accounts = [account({ harness: "claude", kind: "token" })];
  operationStatus = {
    id: "recovered-switch",
    account_id: "account-a",
    phase: "complete",
    error: null,
  };
  const stopped = agent({
    status: "stopped",
    subscriptionAccountHarness: "claude",
    subscriptionAccountId: "account-a",
  });
  let reads = 0;
  function RestoredProfile() {
    const result = useQuery({
      queryKey: ["managed-agents"],
      initialData: [stopped],
      staleTime: Infinity,
      queryFn: async () => {
        reads += 1;
        return [
          {
            ...stopped,
            status: "running",
            runningSubscriptionAccountId: "account-a",
            runningSubscriptionAccountGeneration: 2,
          },
        ];
      },
    });
    return React.createElement(SubscriptionAccountSelector, {
      agent: result.data[0],
    });
  }
  const view = mount(React.createElement(RestoredProfile));
  await waitFor(() =>
    assert.match(view.container.textContent, /Running: Work/),
  );
  assert.equal(reads, 1);
  assert.equal(
    calls.filter(([command]) => command === "switch_agent_subscription_account")
      .length,
    0,
  );
});

test("browser verification updates the rendered card while Buzz remains unfocused", async () => {
  accounts = [account({ state: "connecting", nickname: null })];
  const view = mount(React.createElement(SubscriptionAccountsSettingsCard));
  await view.findByRole("button", { name: "Cancel login" });
  focusManager.setFocused(false);
  accounts[0].state = "ready";
  accounts[0].attempt_id = null;
  // No click, focus event, manual invalidation or replacement component.
  await view.findByRole("textbox", { name: /Nickname/ }, { timeout: 2500 });
  assert.equal(view.queryByRole("button", { name: "Cancel login" }), null);
  assert.match(view.container.textContent, /Subscription login checked/);
});

test("returning from provider login refreshes before the next polling tick", async () => {
  accounts = [account({ state: "connecting", nickname: null })];
  const view = mount(React.createElement(SubscriptionAccountsSettingsCard));
  await view.findByRole("button", { name: "Cancel login" });
  focusManager.setFocused(false);
  accounts[0].state = "ready";
  await React.act(async () => {
    focusManager.setFocused(true);
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  assert.ok(view.getByRole("textbox", { name: /Nickname/ }));
});

test("provider login precedes verification and nickname; Settings refresh reaches both selectors", async () => {
  const view = mount(
    React.createElement(SubscriptionAccountsSettingsCard),
    React.createElement(SubscriptionAccountSelector, { agent: agent() }),
    React.createElement(SubscriptionAccountSelector, {
      agent: agent(),
      compact: true,
    }),
  );
  const signIn = await view.findByRole("button", {
    name: /Sign in with openai/,
  });
  fireEvent.click(
    view.getByRole("button", { name: "Change subscription account" }),
  );
  assert.equal(view.queryByRole("textbox"), null);
  assert.equal(
    view.getByRole("button", { name: /Sign in with xai/ }).disabled,
    true,
  );
  fireEvent.click(signIn);
  await view.findByRole("button", { name: "Cancel login" });
  assert.equal(view.queryByRole("textbox"), null);
  assert.equal(view.queryAllByRole("option", { name: "Work" }).length, 0);
  accounts[0].state = "ready";
  accounts[0].attempt_id = null;
  await refresh();
  const nickname = await view.findByRole("textbox", { name: /Nickname/ });
  fireEvent.change(nickname, { target: { value: "Team account" } });
  fireEvent.click(view.getByRole("button", { name: "Save nickname" }));
  await waitFor(() => assert.equal(accounts[0].nickname, "Team account"));
  await openAccountMenu(
    view,
    view.getAllByRole("button", { name: "Subscription account" })[0],
  );
  await view.findByRole("menuitemradio", { name: "Team account" });
  closeAccountMenu(view);
  await waitFor(() => assert.equal(view.queryByRole("menu"), null));
  // Escape closes the nested popover as well; reopen through the real trigger.
  if (
    view.getAllByRole("button", { name: "Subscription account" }).length === 1
  )
    fireEvent.click(
      view.getByRole("button", { name: "Change subscription account" }),
    );
  await openAccountMenu(
    view,
    view.getAllByRole("button", { name: "Subscription account" }).at(-1),
  );
  await view.findByRole("menuitemradio", { name: "Team account" });
  closeAccountMenu(view);
  assert.deepEqual(
    calls.find(([name]) => name === "begin_subscription_account_login")[1],
    { harness: "codex", accountId: null },
  );
});

test("expired selected account stays visible; reconnect preserves id and nickname", async () => {
  accounts = [
    account({ state: "needs_login", last_error: "Subscription login expired" }),
  ];
  const view = mount(
    React.createElement(SubscriptionAccountsSettingsCard),
    React.createElement(SubscriptionAccountSelector, {
      agent: agent({ subscriptionAccountId: "account-a" }),
    }),
  );
  const picker = await view.findByRole("button", {
    name: "Subscription account",
  });
  await waitFor(() => assert.equal(picker.disabled, false));
  assert.match(picker.textContent, /Work/);
  await openAccountMenu(view, picker);
  assert.equal(
    view
      .getByRole("menuitemradio", { name: /Work — needs login/ })
      .getAttribute("aria-disabled"),
    "true",
  );
  closeAccountMenu(view);
  fireEvent.click(view.getByRole("button", { name: "Reconnect" }));
  await view.findByRole("button", { name: "Cancel login" });
  assert.deepEqual(
    calls.find(([name]) => name === "begin_subscription_account_login")[1],
    { harness: "codex", accountId: "account-a" },
  );
  assert.equal(accounts[0].nickname, "Work");
});

test("cancelled login remains recoverable and never reveals nickname", async () => {
  accounts = [account({ nickname: null, state: "connecting" })];
  const view = mount(React.createElement(SubscriptionAccountsSettingsCard));
  fireEvent.click(await view.findByRole("button", { name: "Cancel login" }));
  await view.findByRole("button", { name: "Reconnect" });
  assert.equal(view.queryByRole("textbox"), null);
  assert.match(view.container.textContent, /Login cancelled/);
});

test("failed cancellation stays visible and retryable", async () => {
  accounts = [account({ nickname: null, state: "connecting" })];
  failCancel = true;
  const view = mount(React.createElement(SubscriptionAccountsSettingsCard));
  fireEvent.click(await view.findByRole("button", { name: "Cancel login" }));
  await view.findByRole("alert");
  assert.equal(
    view.getByRole("button", { name: "Cancel login" }).disabled,
    false,
  );
});

test("experiment off preserves selected account, reconnect and explicit ambient recovery", async () => {
  accounts = [account()];
  enabled = false;
  const view = mount(
    React.createElement(SubscriptionAccountsSettingsCard),
    React.createElement(SubscriptionAccountSelector, {
      agent: agent({ subscriptionAccountId: "account-a" }),
    }),
  );
  const signIn = await view.findByRole("button", {
    name: /Sign in with openai/,
  });
  assert.equal(signIn.disabled, true);
  assert.equal(view.getByRole("button", { name: "Reconnect" }).disabled, false);
  const picker = view.getByRole("button", { name: "Subscription account" });
  assert.match(picker.textContent, /Work/);
  await chooseAccount(view, /Ambient — harness/);
  await waitFor(() =>
    assert.ok(
      calls.some(
        ([name, args]) =>
          name === "switch_agent_subscription_account" &&
          args.accountId === null,
      ),
    ),
  );
});

test("active work in another channel blocks switching without queuing", async () => {
  accounts = [account()];
  reportChannelBotTyping("other-channel", [pubkey]);
  const view = mount(
    React.createElement(SubscriptionAccountSelector, { agent: agent() }),
  );
  const picker = await view.findByRole("button", {
    name: "Subscription account",
  });
  assert.equal(picker.disabled, true);
  assert.match(view.container.textContent, /Finish the active turn/);
  assert.equal(calls.filter(([name]) => name.includes("switch")).length, 0);
});

test("restart failure stays visible and does not claim selected account is running", async () => {
  accounts = [account()];
  failSwitch = true;
  const view = mount(
    React.createElement(SubscriptionAccountSelector, {
      agent: agent({ runningSubscriptionAccountGeneration: 0 }),
    }),
  );
  await chooseAccount(view, "Work");
  await view.findByRole("alert");
  assert.match(view.container.textContent, /runtime restart failed/);
  assert.match(view.container.textContent, /Running: Ambient/);
  assert.doesNotMatch(view.container.textContent, /Running: Work/);
});

test("legacy and missing selections cannot be mistaken for ambient", async () => {
  const view = mount(
    React.createElement(SubscriptionAccountSelector, {
      agent: agent({ claudeAccountId: "old-id" }),
    }),
    React.createElement(SubscriptionAccountSelector, {
      agent: agent({
        pubkey: "b".repeat(64),
        subscriptionAccountId: "missing",
      }),
    }),
  );
  const [legacy, missing] = await view.findAllByRole("button", {
    name: "Subscription account",
  });
  await waitFor(() => assert.equal(legacy.disabled, false));

  assert.match(legacy.textContent, /Legacy/);
  assert.match(missing.textContent, /unavailable/);
});

test("referenced account removal error leaves recovery controls intact", async () => {
  accounts = [account()];
  failRemove = true;
  const view = mount(React.createElement(SubscriptionAccountsSettingsCard));
  fireEvent.click(await view.findByRole("button", { name: "Remove" }));
  fireEvent.click(view.getByRole("button", { name: "Sign out and remove" }));
  await view.findByRole("alert");
  assert.equal(view.getByRole("button", { name: "Reconnect" }).disabled, false);
  assert.equal(accounts.length, 1);
});

test("experiment switch persists through backend and updates the shared query", async () => {
  const view = mount(React.createElement(SubscriptionAccountsExperimentRow));
  const toggle = await view.findByRole("switch");
  await waitFor(() => assert.equal(toggle.disabled, false));
  fireEvent.click(toggle);
  await waitFor(() => assert.equal(enabled, false));
  assert.equal(client.getQueryData(["subscription-accounts-enabled"]), false);
});

test("authentication recovery requires explicit local resume with partial-work warning", async () => {
  accounts = [account()];
  enabled = false;
  const view = mount(
    React.createElement(SubscriptionAccountSelector, {
      agent: agent({
        subscriptionAccountId: "account-a",
        subscriptionAccountError: "subscription_auth_required",
      }),
    }),
  );
  fireEvent.click(
    await view.findByRole("button", { name: "Resume retained work" }),
  );
  assert.match(
    view.container.textContent,
    /Earlier partial work may already have run/,
  );
  assert.equal(
    calls.filter(([name]) => name === "resume_agent_subscription_work").length,
    0,
  );
  fireEvent.click(view.getByRole("button", { name: "Resume now" }));
  await waitFor(() =>
    assert.deepEqual(
      calls.find(([name]) => name === "resume_agent_subscription_work")[1],
      { pubkey },
    ),
  );
});

test("auth-held custom harness retains resume access without inventing account support", async () => {
  const view = mount(
    React.createElement(SubscriptionAccountSelector, {
      agent: agent({
        subscriptionAccountHarness: null,
        subscriptionAccountError: "subscription_auth_required",
      }),
    }),
  );
  await view.findByRole("button", { name: "Resume retained work" });
  assert.equal(
    view.getByRole("button", { name: "Subscription account" }).disabled,
    true,
  );
});

test("token setup crosses only metadata IPC, nickname precedes selection, renewal works with experiment off", async () => {
  const view = mount(
    React.createElement(SubscriptionAccountsSettingsCard),
    React.createElement(SubscriptionAccountSelector, {
      agent: agent({ subscriptionAccountHarness: "claude" }),
    }),
  );
  fireEvent.click(
    await view.findByRole("button", { name: "Add Claude long-lived token" }),
  );
  await view.findByRole("button", { name: "Cancel token setup" });
  assert.equal(view.queryByRole("textbox"), null);
  assert.deepEqual(
    calls.find(([name]) => name === "begin_subscription_account_token")[1],
    { accountId: null },
  );
  const minted = Math.floor(Date.now() / 1000);
  Object.assign(accounts[0], {
    state: "ready",
    minted_at: minted,
    expires_at: minted + 365 * 86400,
  });
  await refresh();
  const nickname = await view.findByRole("textbox", { name: /Nickname/ });
  fireEvent.change(nickname, { target: { value: "Shared team" } });
  fireEvent.click(view.getByRole("button", { name: "Save nickname" }));
  await waitFor(() => assert.equal(accounts[0].nickname, "Shared team"));
  await chooseAccount(view, "Shared team — Expires in 365 days");
  assert.equal(
    calls.filter(([name]) => name === "switch_agent_subscription_account")
      .length,
    0,
  );
  fireEvent.click(
    await view.findByRole("button", { name: "Switch this agent" }),
  );
  await waitFor(() =>
    assert.ok(
      calls.some(
        ([name, args]) =>
          name === "switch_agent_subscription_account" &&
          args.accountId === "account-a",
      ),
    ),
  );
  enabled = false;
  await React.act(() =>
    client.invalidateQueries({ queryKey: ["subscription-accounts-enabled"] }),
  );
  await waitFor(() =>
    assert.equal(
      view.getByRole("button", { name: "Add Claude long-lived token" })
        .disabled,
      true,
    ),
  );
  fireEvent.click(view.getByRole("button", { name: "Renew" }));
  await view.findByRole("button", { name: "Cancel token setup" });
  assert.deepEqual(
    calls
      .filter(([name]) => name === "begin_subscription_account_token")
      .at(-1)[1],
    { accountId: "account-a" },
  );
  assert.equal(accounts[0].nickname, "Shared team");
  assert.equal(
    calls.filter(([name]) => name === "begin_subscription_account_login")
      .length,
    0,
  );
});

test("expired token stays selected, cannot be selected anew and offers renewal without stop instructions", async () => {
  accounts = [account({ kind: "token", expires_at: 1, minted_at: 0 })];
  const view = mount(
    React.createElement(SubscriptionAccountSelector, {
      agent: agent({
        subscriptionAccountId: "account-a",
        subscriptionAccountError: "subscription_auth_required",
      }),
    }),
  );
  await openAccountMenu(view);
  const option = await view.findByRole("menuitemradio", {
    name: "Work — Expired",
  });
  assert.equal(option.getAttribute("aria-disabled"), "true");
  closeAccountMenu(view);
  assert.match(
    view.getByRole("button", { name: "Subscription account" }).textContent,
    /Work/,
  );
  assert.match(view.container.textContent, /Renew in Settings/);
  assert.doesNotMatch(view.container.textContent, /Stop the agent/);
  assert.equal(
    view.getByTestId("selected-token-expiry").textContent,
    "Expired",
  );
});

test("token card keeps a renewal notice without relying on notification permission", async () => {
  accounts = [
    account({ kind: "token", expires_at: Date.now() / 1000 + 29 * 86400 }),
  ];
  enabled = false;
  const view = mount(React.createElement(SubscriptionAccountsSettingsCard));
  const notice = await view.findByTestId("token-renewal-notice-account-a");
  assert.match(notice.textContent, /within 30 days.*Choose Renew/);
  assert.equal(view.getByRole("button", { name: "Renew" }).disabled, false);
  assert.match(view.container.textContent, /local OS notifications/);
  assert.match(view.container.textContent, /while Desktop is running/);
  assert.doesNotMatch(view.container.textContent, /owner DM/);
  accounts[0].expires_at = 1;
  await refresh();
  await waitFor(() =>
    assert.match(notice.textContent, /has expired.*Choose Renew/),
  );
});

test("Claude setup offers only tokens and retired login retains removal", async () => {
  accounts = [
    account({ harness: "claude", provider: "anthropic", kind: "login" }),
  ];
  const view = mount(React.createElement(SubscriptionAccountsSettingsCard));
  await view.findByRole("button", { name: "Add Claude long-lived token" });
  assert.equal(
    view.queryByRole("button", { name: /Sign in with anthropic/ }),
    null,
  );
  assert.equal(view.queryByRole("button", { name: "Reconnect" }), null);
  assert.match(view.container.textContent, /sign-in method has been retired/);
  fireEvent.click(view.getByRole("button", { name: "Remove" }));
  fireEvent.click(view.getByRole("button", { name: "Sign out and remove" }));
  await waitFor(() => assert.equal(accounts.length, 0));
});

test("active Claude work can switch only after the native single-agent confirmation", async () => {
  accounts = [
    account({
      harness: "claude",
      provider: "anthropic",
      kind: "token",
      expires_at: Math.floor(Date.now() / 1000) + 86400,
    }),
  ];
  reportChannelBotTyping("other-channel", [pubkey]);
  const view = mount(
    React.createElement(SubscriptionAccountSelector, {
      agent: agent({ subscriptionAccountHarness: "claude", status: "running" }),
    }),
  );
  await chooseAccount(view, /Work/);
  assert.equal(
    calls.filter(([name]) => name === "switch_agent_subscription_account")
      .length,
    0,
  );
  assert.match(
    (await view.findByRole("alertdialog")).textContent,
    /Stopped runtimes stay stopped/,
  );
  fireEvent.click(view.getByRole("button", { name: "Cancel" }));
  assert.equal(
    calls.filter(([name]) => name === "switch_agent_subscription_account")
      .length,
    0,
  );
  await chooseAccount(view, /Work/);
  fireEvent.click(
    await view.findByRole("button", { name: "Switch this agent" }),
  );
  await waitFor(() =>
    assert.equal(
      calls.filter(([name]) => name === "switch_agent_subscription_account")
        .length,
      1,
    ),
  );
});

test("a persisted failed switch exposes retry and keeps the destination selectable", async () => {
  accounts = [
    account({
      harness: "claude",
      provider: "anthropic",
      kind: "token",
      expires_at: Math.floor(Date.now() / 1000) + 86400,
    }),
  ];
  operationStatus = {
    id: "op",
    account_id: "account-a",
    phase: "restarting",
    error: "Selected account could not start",
  };
  const view = mount(
    React.createElement(SubscriptionAccountSelector, {
      agent: agent({
        subscriptionAccountHarness: "claude",
        subscriptionAccountId: "account-a",
      }),
    }),
  );
  await view.findByRole("button", { name: "Retry account switch" });
  assert.equal(
    view.getByRole("button", { name: "Subscription account" }).disabled,
    false,
  );
  assert.match(view.container.textContent, /Selected account could not start/);
  fireEvent.click(view.getByRole("button", { name: "Retry account switch" }));
  await waitFor(() =>
    assert.equal(
      calls.filter(([name]) => name === "switch_agent_subscription_account")
        .length,
      1,
    ),
  );
});
