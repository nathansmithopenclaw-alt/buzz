import assert from "node:assert/strict";
import test from "node:test";
import { instanceModelSelectionHandlers } from "./instanceModelSelectionHandlers.ts";
import { connectionModelValue } from "@/features/model-connections/modelChoices.ts";

function fixture() {
  const writes = [],
    efforts = [],
    runtimes = [];
  const source = {
    connection: {
      id: "source-two",
      models: ["exact/model"],
      qualification: {
        revision: 1,
        model: "exact/model",
        protocols: [
          {
            protocol: "anthropic_messages",
            tool_round_trip: true,
            effort_levels: ["low", "medium", "xhigh"],
          },
        ],
      },
    },
    model: "exact/model",
  };
  const selection = {
    model: "hosted[high]",
    provider: "openai",
    envVars: { OPENAI_API_KEY: "inactive" },
    isCustomModelEditing: false,
    isCustomProviderEditing: false,
  };
  const handlers = instanceModelSelectionHandlers({
    selection,
    applySelection: (next) => writes.push(next),
    handleRuntimeDropdownChange: (runtime) => runtimes.push(runtime),
    selectedRuntime: {
      id: "test-harness",
      modelConnection: {
        protocol: "anthropic_messages",
        defaultEffort: "medium",
      },
    },
    selectedRuntimeId: "test-harness",
    connections: {
      resolve: (value) =>
        value === connectionModelValue("source-two", "exact/model")
          ? source
          : null,
    },
    instanceEffort: { value: null, onChange: (value) => efforts.push(value) },
    savedEffort: null,
  });
  return { handlers, writes, efforts, runtimes };
}

test("instance picker applies exact source and raw model with qualified initial effort", () => {
  const f = fixture();
  f.handlers.handleModelDropdownChange(
    connectionModelValue("source-two", "exact/model"),
  );
  assert.equal(f.writes.length, 1);
  assert.equal(f.writes[0].model, "exact/model");
  assert.deepEqual(f.writes[0].envVars, {
    OPENAI_API_KEY: "inactive",
    BUZZ_MODEL_CONNECTION_ID: "source-two",
  });
  assert.deepEqual(f.efforts, ["medium"]);
});

test("unresolved connection selection never becomes a hosted model", () => {
  const f = fixture();
  f.handlers.handleModelDropdownChange(
    connectionModelValue("missing", "exact/model"),
  );
  assert.deepEqual(f.writes, []);
  assert.deepEqual(f.efforts, []);
});

test("instance hosted provider transition retains the relay runtime rule", () => {
  const f = fixture();
  f.handlers.handleProviderDropdownChange("relay-mesh");
  assert.deepEqual(f.runtimes, ["buzz-agent"]);
  assert.equal(f.writes[0].model, "auto");
});
