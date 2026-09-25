import { test } from "node:test";
import assert from "node:assert/strict";
import { cleanupOptions, cleanupValue } from "./cleanupOptions.ts";

test("Spark models use the Local models group without changing the connection wire type", () => {
  const options = cleanupOptions(
    {
      agentOpenRouter: true,
      openRouter: [{ id: "google/gemini-3.8-flash", name: "Gemini 3.8 Flash" }],
      localConnectionIds: ["spark"],
      error: null,
    },
    [{ id: "spark", name: "Spark 1 · Qwen", models: ["Qwen"] }],
    [{ id: "claude-haiku-4-5" }],
    { kind: "off" },
  );
  const local = options.find(
    (option) => option.label === "Spark 1 · Qwen · Qwen",
  );
  assert.equal(local.group, "Local models");
  assert.deepEqual(JSON.parse(local.value), {
    kind: "connection",
    connectionId: "spark",
    model: "Qwen",
  });
  assert.ok(
    options.some(
      (option) =>
        option.label === "Gemini 3.8 Flash" && option.badge === "Recommended",
    ),
  );
});

test("an unavailable saved model stays selected and custom IDs remain possible", () => {
  const current = {
    kind: "connection",
    connectionId: "removed",
    model: "custom",
  };
  const options = cleanupOptions(undefined, [], [], current);
  assert.ok(options.some((option) => option.value === cleanupValue(current)));
  assert.ok(options.some((option) => option.value.startsWith("custom:")));
});
