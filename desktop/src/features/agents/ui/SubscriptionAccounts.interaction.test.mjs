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
let accounts, enabled, failSwitch, failRemove, failCancel;
const capabilities = [
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
const { render, cleanup, fireEvent, waitFor, within } = await import(
  "@testing-library/react"
);
const { focusManager, QueryClient, QueryClientProvider } = await import(
  "@tanstack/react-query"
);
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
async function refresh() {
  await React.act(async () =>
    client.invalidateQueries({ queryKey: ["subscription-accounts"] }),
  );
}
beforeEach(() => {
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
  await waitFor(() =>
    assert.equal(
      view.getAllByRole("option", { name: "Team account" }).length,
      2,
    ),
  );
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
  const picker = await view.findByRole("combobox");
  await waitFor(() => assert.equal(picker.disabled, false));
  assert.equal(picker.value, "account-a");
  assert.equal(
    view.getByRole("option", { name: /Work — needs login/ }).disabled,
    true,
  );
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
  const picker = view.getByRole("combobox");
  assert.equal(picker.value, "account-a");
  fireEvent.change(picker, { target: { value: "" } });
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
  const picker = await view.findByRole("combobox");
  await view.findByRole("option", { name: "Work" });
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
  await view.findByRole("option", { name: "Work" });
  fireEvent.change(view.getByRole("combobox"), {
    target: { value: "account-a" },
  });
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
  const [legacy, missing] = await view.findAllByRole("combobox");
  await waitFor(() => assert.equal(legacy.disabled, false));
  assert.equal(legacy.value, "__legacy__");
  assert.equal(missing.value, "missing");
  assert.match(
    within(legacy).getByRole("option", { selected: true }).textContent,
    /Legacy/,
  );
  assert.match(
    within(missing).getByRole("option", { selected: true }).textContent,
    /unavailable/,
  );
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
  assert.equal(view.getByRole("combobox").disabled, true);
});
