import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAgentMemorySources,
  indexAgentDeployments,
} from "./agentDeployments.ts";

function agent(pubkey, personaId, name = pubkey) {
  return { name, personaId, pubkey };
}

test("indexAgentDeployments retains every deployment for a definition", () => {
  const deployments = indexAgentDeployments([
    agent("one", "persona-a", "One"),
    agent("two", "persona-a", "Two"),
    agent("three", "persona-b", "Three"),
  ]);

  assert.deepEqual(deployments.get("persona-a"), [
    { name: "One", pubkey: "one" },
    { name: "Two", pubkey: "two" },
  ]);
  assert.deepEqual(deployments.get("persona-b"), [
    { name: "Three", pubkey: "three" },
  ]);
});

test("indexAgentDeployments never assigns unlinked or unknown-definition instances", () => {
  const agents = [
    agent("unlinked", null),
    agent("unknown", "missing-definition"),
  ];
  const deployments = indexAgentDeployments(agents);

  assert.equal(deployments.has(""), false);
  assert.deepEqual(deployments.get("missing-definition"), [
    { name: "unknown", pubkey: "unknown" },
  ]);
  assert.equal(agents.length, 2);
});

test("buildAgentMemorySources distinguishes deployments with the same visible name", () => {
  assert.deepEqual(
    buildAgentMemorySources([
      agent("0123456789abcdef", "persona-a", "Reviewer"),
      agent("fedcba9876543210", "persona-a", " reviewer "),
      agent("unique", "persona-a", "Operator"),
    ]),
    [
      { name: "Reviewer · 012345…abcdef", pubkey: "0123456789abcdef" },
      { name: "reviewer · fedcba…543210", pubkey: "fedcba9876543210" },
      { name: "Operator", pubkey: "unique" },
    ],
  );
});
