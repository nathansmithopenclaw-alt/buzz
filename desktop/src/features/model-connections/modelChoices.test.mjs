import assert from "node:assert/strict";
import test from "node:test";
import {
  connectionModelChoices,
  connectionModelValue,
  resolveConnectionModelValue,
  connectionEffortLevels,
  connectionSelectionError,
  connectionEffortError,
  connectionAccountReason,
} from "./modelChoices.ts";

const support = {
  protocol: "anthropic_messages",
  setupReason: null,
  defaultEffort: "medium",
};
const row = (id, name = id) => ({
  id,
  name,
  models: ["Qwen/model"],
  qualification: {
    revision: 1,
    model: "Qwen/model",
    protocols: [
      {
        protocol: "anthropic_messages",
        tool_round_trip: true,
        reason: null,
        effort_levels: ["low", "medium", "xhigh"],
      },
    ],
  },
});

test("duplicate model IDs and Claude alias labels retain exact source identity", () => {
  for (const alias of ["default", "opus", "sonnet", "haiku", "fable"]) {
    const rows = [row("one", alias), row("two", alias)];
    const choices = connectionModelChoices(rows, support);
    assert.notEqual(choices[0].value, choices[1].value);
    assert.equal(
      resolveConnectionModelValue(choices[1].value, rows).connection.id,
      "two",
    );
    assert.equal(
      resolveConnectionModelValue(choices[1].value, rows).model,
      "Qwen/model",
    );
    assert.equal(choices[1].label, "Qwen/model");
    assert.equal(choices[1].group, alias);
  }
});

test("saved choice never falls back when its connection or model disappears", () => {
  const value = connectionModelValue("one", "Qwen/model");
  assert.equal(resolveConnectionModelValue(value, []), null);
  assert.equal(
    resolveConnectionModelValue(value, [{ ...row("one"), models: [] }]),
    null,
  );
  for (const malformed of [
    "saved-connection:{}",
    "saved-connection:[]",
    'saved-connection:"ab"',
    "saved-connection:invalid",
  ])
    assert.equal(resolveConnectionModelValue(malformed, [row("one")]), null);
});

test("unknown harnesses omit rows; actionable setup reasons disable rows", () => {
  assert.deepEqual(connectionModelChoices([row("one")], null), []);
  const reason = "Install the Pi ACP adapter to use this connection";
  const [choice] = connectionModelChoices([row("one")], {
    protocol: null,
    setupReason: reason,
    defaultEffort: null,
  });
  assert.equal(choice.disabled, true);
  assert.equal(choice.badge, reason);
});

test("unqualified model has no effort values and qualified values never include inherit", () => {
  const unqualified = { ...row("one"), qualification: null };
  assert.deepEqual(
    connectionEffortLevels(unqualified, "Qwen/model", support),
    [],
  );
  assert.equal(
    connectionModelChoices([unqualified], support)[0].badge,
    "Test this model in Settings → AI",
  );
  assert.deepEqual(connectionEffortLevels(row("one"), "Qwen/model", support), [
    "low",
    "medium",
    "xhigh",
  ]);
  assert.deepEqual(
    connectionEffortLevels(row("one"), "different-model", support),
    [],
  );
});

test("loading, removed and unqualified saved sources block Save with a reason", () => {
  const value = connectionModelValue("one", "Qwen/model");
  const options = connectionModelChoices([row("one")], support);
  assert.equal(connectionSelectionError(value, options, false, null), null);
  assert.match(connectionSelectionError(value, options, true, null), /Loading/);
  assert.equal(
    connectionSelectionError(value, options, false, "Read failed"),
    "Read failed",
  );
  assert.match(
    connectionSelectionError(value, [], false, null),
    /connection “one”.*unavailable/,
  );
  assert.match(
    connectionSelectionError(value, [], false, null, "My server"),
    /connection “My server”/,
  );
  assert.match(
    connectionSelectionError(
      value,
      connectionModelChoices([{ ...row("one"), qualification: null }], support),
      false,
      null,
    ),
    /Test/,
  );
  assert.equal(connectionSelectionError(null, [], true, "Read failed"), null);
});

test("connection effort validation does not inherit or remap unsupported high", () => {
  const levels = ["low", "medium", "xhigh"];
  for (const value of levels)
    assert.equal(connectionEffortError(levels, value), null);
  for (const value of ["", "default", "high"])
    assert.match(connectionEffortError(levels, value), /tested/);
  assert.equal(connectionEffortError([], "high"), null);
});

test("connection selection requires the existing account detach flow", () => {
  for (const agent of [
    { subscriptionAccountId: "account" },
    { subscriptionAccountInheritDefault: true },
  ])
    assert.match(
      connectionAccountReason(agent),
      /Remove.*subscription account/,
    );
  assert.equal(connectionAccountReason(undefined), null);
  assert.equal(
    connectionAccountReason({
      subscriptionAccountId: null,
      subscriptionAccountInheritDefault: false,
    }),
    null,
  );
});
