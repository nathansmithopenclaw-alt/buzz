import assert from "node:assert/strict";
import { after, afterEach, test } from "node:test";
import { JSDOM } from "jsdom";
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  localStorage: dom.window.localStorage,
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});
for (const key of Object.getOwnPropertyNames(dom.window)) {
  const value = dom.window[key];
  if (
    typeof value === "function" &&
    /^(HTML|SVG)|Element$|Event$|EventTarget$|^Node|^Document|Observer$/.test(
      key,
    )
  )
    globalThis[key] = value;
}
globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
window.matchMedia = () => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
});
globalThis.requestAnimationFrame = (cb) => setTimeout(cb, 0);
window.__TAURI_INTERNALS__ = {
  invoke: async (command) => {
    if (command === "list_managed_agents") return [];
    if (command === "plugin:event|listen") return 1;
    return null;
  },
  transformCallback: () => 1,
};
globalThis.__TAURI_INTERNALS__ = window.__TAURI_INTERNALS__;
const React = await import("react");
const { render, fireEvent, cleanup, screen, waitFor } = await import(
  "@testing-library/react"
);
const { QueryClient, QueryClientProvider } = await import(
  "@tanstack/react-query"
);
const { SummaryBuilder } = await import("./SummaryBuilder.tsx");
const { ManagedAgentPickerDialog } = await import(
  "@/features/agents/ui/ManagedAgentPickerDialog.tsx"
);
const { ThemeProvider } = await import("@/shared/theme/ThemeProvider.tsx");
const clients = [];
function wrap(element) {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, gcTime: Infinity },
    },
  });
  clients.push(client);
  client.setQueryData(
    ["managed-agents"],
    [
      { pubkey: "aa", name: "Grok", avatarUrl: null },
      { pubkey: "bb", name: "Nectar", avatarUrl: null },
    ],
  );
  return React.createElement(
    QueryClientProvider,
    { client },
    React.createElement(ThemeProvider, null, element),
  );
}
afterEach(() => {
  cleanup();
  for (const client of clients.splice(0)) client.clear();
});
after(() => dom.window.close());
test("guided workflow retains source choices and saves only at the final step", () => {
  let saves = 0;
  function Form() {
    const [sourceConfig, setSourceConfig] = React.useState({
      autoInclude: false,
      include: ["c", "d"],
      exclude: [],
    });
    return React.createElement(SummaryBuilder, {
      agents: [],
      conversations: [
        { id: "c", name: "General", kind: "channel", archived: false },
        { id: "d", name: "Grok, Nectar", kind: "dm", archived: false },
      ],
      sourceConfig,
      onSourceConfigChange: setSourceConfig,
      schedule: { on: "schedule", cron: "0 9 * * *" },
      timezone: "UTC",
      reporterMode: "existing",
      reporterPubkey: "aa",
      nectarExists: false,
      runtimeValid: true,
      onRuntimeDraftChange() {},
      saving: false,
      error: null,
      onScheduleChange() {},
      onReporterModeChange() {},
      onReporterChange() {},
      onSave() {
        saves++;
      },
    });
  }
  render(wrap(React.createElement(Form)));
  assert.equal(screen.queryByRole("switch", { name: "Grok, Nectar" }), null);
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("switch", { name: "Grok, Nectar" }));
  assert.equal(
    screen
      .getByRole("switch", { name: "Grok, Nectar" })
      .getAttribute("aria-checked"),
    "false",
  );
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  assert.equal(saves, 0);
  fireEvent.click(screen.getByRole("button", { name: "Back" }));
  assert.equal(
    screen
      .getByRole("switch", { name: "Grok, Nectar" })
      .getAttribute("aria-checked"),
    "false",
  );
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Save Summary" }));
  assert.equal(saves, 1);
});
test("single-agent picker uses the managed roster and replaces the draft selection", () => {
  let confirmed;
  function Picker() {
    const [selected, setSelected] = React.useState(["aa"]);
    return React.createElement(ManagedAgentPickerDialog, {
      open: true,
      onOpenChange() {},
      title: "Summary agent",
      description: "Choose an agent",
      selected,
      onSelectionChange: setSelected,
      multiple: false,
      onConfirm() {
        confirmed = selected;
      },
    });
  }
  render(wrap(React.createElement(Picker)));
  fireEvent.click(screen.getByRole("checkbox", { name: "Nectar" }));
  assert.equal(
    screen.getByRole("checkbox", { name: "Grok" }).getAttribute("aria-checked"),
    "false",
  );
  assert.equal(confirmed, undefined);
  fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
  assert.deepEqual(confirmed, ["bb"]);
});

test("Nectar seeds the saved harness and model and discovers native tuning through Edit Agent", async () => {
  const { SummaryRuntimeFields } = await import("./SummaryRuntimeFields.tsx");
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, gcTime: Infinity },
    },
  });
  clients.push(client);
  client.setQueryData(["globalAgentConfig"], {
    env_vars: {},
    preferred_runtime: "codex",
    model: "gpt-5.4",
    provider: null,
  });
  const runtime = {
    id: "codex",
    label: "Codex",
    availability: "available",
    command: "codex",
    binaryPath: "/usr/local/bin/codex",
    defaultArgs: [],
    modelEnvVar: "CODEX_MODEL",
    providerEnvVar: null,
    effortModelQualifier: true,
    nativeFastMode: true,
  };
  client.setQueryData(
    ["acp-runtimes"],
    [
      {
        ...runtime,
        id: "buzz-agent",
        label: "Buzz Agent",
        command: "buzz-agent",
      },
      runtime,
      {
        ...runtime,
        id: "deepseek",
        label: "DeepSeek Harness",
        command: "dsh",
        effortModelQualifier: false,
        nativeFastMode: false,
        modelEnvVar: "DSH_MODEL",
        providerEnvVar: "DSH_PROVIDER",
        providerOptions: [
          ["deepseek-hosted", "DeepSeek API"],
          ["deepseek-compatible", "Local / OpenAI-compatible endpoint"],
        ],
      },
    ],
  );
  const calls = [];
  const previous = window.__TAURI_INTERNALS__.invoke;
  window.__TAURI_INTERNALS__.invoke = async (command, payload) => {
    if (command === "discover_agent_models") {
      calls.push(payload);
      return {
        supportsSwitching: true,
        models:
          payload.input.agentCommand === "dsh"
            ? [{ id: "Qwen/Qwen3.8-27B", name: "Qwen" }]
            : [{ id: "gpt-5.4", name: "GPT-5.4" }],
        effortConfigId: "reasoning_effort",
        effortOptions: [
          { id: "low", name: "Low" },
          { id: "high", name: "High" },
        ],
        fastModeConfigId: "fast-mode",
        fastModeEnabled: false,
      };
    }
    if (
      command === "get_baked_build_env" ||
      command === "get_baked_build_env_keys"
    )
      return [];
    return previous(command, payload);
  };
  let draft;
  const changed = (value) => {
    draft = value;
  };
  render(
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(
        ThemeProvider,
        null,
        React.createElement(SummaryRuntimeFields, {
          disabled: false,
          onChange: changed,
        }),
      ),
    ),
  );
  try {
    await waitFor(() => assert.equal(draft?.model, "gpt-5.4"));
    assert.equal(draft.runtime.id, "codex");
    assert.match(
      document.querySelector("#summary-nectar-harness").textContent,
      /Codex/,
    );
    assert.match(
      document.querySelector("#summary-nectar-model").textContent,
      /GPT-5.4/,
    );
    assert.equal(calls.at(-1).input.agentCommand, "codex");
    assert.equal(calls.at(-1).input.model, "gpt-5.4");
    assert.equal(document.querySelector("select#summary-nectar-model"), null);
    fireEvent.click(screen.getByTestId("definition-agent-fast-mode-toggle"));
    await waitFor(() => assert.equal(draft.envVars.BUZZ_ACP_FAST_MODE, "true"));
    fireEvent.keyDown(screen.getByLabelText("Harness"), { key: "ArrowDown" });
    fireEvent.click(
      await screen.findByRole("menuitemradio", { name: /DeepSeek Harness/ }),
    );
    await waitFor(() => assert.equal(calls.at(-1).input.agentCommand, "dsh"));
    assert.equal(calls.at(-1).input.model, undefined);
    fireEvent.keyDown(screen.getByLabelText("LLM provider"), {
      key: "ArrowDown",
    });
    assert.ok(
      await screen.findByRole("menuitemradio", { name: "DeepSeek API" }),
    );
    assert.equal(screen.queryByRole("button", { name: "OpenAI" }), null);
    fireEvent.click(
      screen.getByRole("menuitemradio", {
        name: "Local / OpenAI-compatible endpoint",
      }),
    );
    fireEvent.change(screen.getByLabelText("Model endpoint URL"), {
      target: { value: "http://127.0.0.1:8010/v1" },
    });
    fireEvent.change(screen.getByLabelText("Endpoint API key (optional)"), {
      target: { value: "fixture-token" },
    });
    await waitFor(() =>
      assert.equal(screen.getByLabelText("Model").disabled, false),
    );
    fireEvent.keyDown(screen.getByLabelText("Model"), { key: "ArrowDown" });
    fireEvent.click(await screen.findByRole("menuitemradio", { name: /Qwen/ }));
    await waitFor(() => assert.equal(draft?.model, "Qwen/Qwen3.8-27B"));
    assert.equal(draft.provider, "deepseek-compatible");
    assert.equal(draft.envVars.DSH_LLM_BASE_URL, "http://127.0.0.1:8010/v1");
    assert.equal(draft.envVars.DSH_API_KEY, "fixture-token");
    fireEvent.change(screen.getByLabelText("Model endpoint URL"), {
      target: { value: "http://127.0.0.1:8011/v1" },
    });
    await waitFor(() => assert.equal(draft, null));
  } finally {
    window.__TAURI_INTERNALS__.invoke = previous;
  }
});

test("Nectar keeps delayed discovery active across unrelated parent renders", async () => {
  const { SummaryRuntimeFields } = await import("./SummaryRuntimeFields.tsx");
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, gcTime: Infinity },
    },
  });
  clients.push(client);
  client.setQueryData(["globalAgentConfig"], {
    env_vars: {},
    preferred_runtime: "claude",
    model: "claude-opus-4-8",
    provider: null,
  });
  client.setQueryData(
    ["acp-runtimes"],
    [
      {
        id: "claude",
        label: "Claude Code",
        availability: "available",
        command: "claude",
        binaryPath: "/usr/local/bin/claude",
        defaultArgs: [],
        modelEnvVar: "ANTHROPIC_MODEL",
        providerEnvVar: null,
      },
    ],
  );
  const previous = window.__TAURI_INTERNALS__.invoke;
  const pending = [];
  window.__TAURI_INTERNALS__.invoke = async (command, payload) => {
    if (command === "discover_agent_models") {
      return new Promise((resolve) =>
        pending.push({ resolve, input: payload.input }),
      );
    }
    if (
      command === "get_baked_build_env" ||
      command === "get_baked_build_env_keys"
    )
      return [];
    return previous(command, payload);
  };
  let draft;
  const changed = (value) => {
    draft = value;
  };
  function Parent() {
    const [tick, setTick] = React.useState(0);
    return React.createElement(
      React.Fragment,
      null,
      React.createElement(
        "button",
        { onClick: () => setTick(tick + 1) },
        `Unrelated ${tick}`,
      ),
      React.createElement(SummaryRuntimeFields, {
        disabled: false,
        onChange: changed,
      }),
    );
  }
  const response = {
    supportsSwitching: true,
    models: [{ id: "claude-opus-4-8", name: "Opus" }],
  };
  try {
    render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(ThemeProvider, null, React.createElement(Parent)),
      ),
    );
    await waitFor(() => assert.ok(pending.length > 0));
    fireEvent.click(screen.getByRole("button", { name: "Unrelated 0" }));
    fireEvent.click(screen.getByRole("button", { name: "Unrelated 1" }));
    assert.equal(
      pending.length,
      1,
      "unchanged discovery must not be restarted by loading or parent renders",
    );
    assert.equal(draft, null);
    pending[0].resolve(response);
    await waitFor(() => assert.equal(draft?.model, "claude-opus-4-8"));
    assert.equal(pending.length, 1);
    assert.equal(draft.runtime.id, "claude");
  } finally {
    cleanup();
    for (const request of pending) request.resolve(response);
    window.__TAURI_INTERNALS__.invoke = previous;
  }
});

test("schedule controls retain local times, selected weekdays and manual-only choices", async () => {
  const { SummaryScheduleFields } = await import("./SummaryScheduleFields.tsx");
  let value;
  function Form() {
    const [schedule, setSchedule] = React.useState({
      on: "schedule",
      cron: "15 8 * * *",
    });
    value = schedule;
    return React.createElement(SummaryScheduleFields, {
      schedule,
      timezone: "America/New_York",
      onChange: setSchedule,
    });
  }
  render(wrap(React.createElement(Form)));
  assert.ok(screen.getByText("Run time (America/New_York)"));
  fireEvent.click(screen.getByRole("radio", { name: "Weekly" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Friday" }));
  assert.equal(value.cron, "15 8 * * 1,5");
  fireEvent.change(document.querySelector('input[type="time"]'), {
    target: { value: "17:45" },
  });
  assert.equal(value.cron, "45 17 * * 1,5");
  fireEvent.click(screen.getByRole("switch", { name: "Run on a schedule" }));
  assert.equal(value, null);
  assert.ok(screen.getByText("Manual only. No scheduled runs."));
  fireEvent.click(screen.getByRole("switch", { name: "Run on a schedule" }));
  assert.equal(value.cron, "45 17 * * 1,5");
  fireEvent.click(screen.getByRole("radio", { name: "Every 30 minutes" }));
  assert.equal(value.interval, "30m");
});
