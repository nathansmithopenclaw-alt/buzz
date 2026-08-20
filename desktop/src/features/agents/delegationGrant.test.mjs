import assert from "node:assert/strict";
import test from "node:test";

import {
  LEFT_ALL_NOTICE,
  agentIsGranted,
  allSubjects,
  emptySubjects,
  grantSubjects,
  subjectSummary,
  subjectsAreEmpty,
  toggleGrantedAgent,
} from "./delegationGrant.ts";

const FIRST = "11".repeat(32);
const SECOND = "22".repeat(32);
const ROSTER = [FIRST, SECOND];

test("v1 grant renders as a one-agent set on the true bits", () => {
  const subjects = grantSubjects({
    granteePubkey: FIRST.toUpperCase(),
    auditChannelId: "7c07e659-3610-42f4-9a5e-1e9973c09da9",
    capabilities: {
      agent_config: false,
      agent_creation: true,
      env_vars: false,
      global_config: false,
      teams: true,
    },
  });
  assert.deepEqual(subjects.agent_creation, {
    scope: "set",
    grantees: [FIRST],
  });
  assert.equal(subjects.agent_config.scope, "none");
  assert.equal(subjectSummary(subjects.agent_creation), "1 agent");
  assert.equal(subjectSummary(subjects.teams), "1 agent");
  assert.equal(subjectSummary(subjects.env_vars), "Off");
});

test("unchecking one name while all is on writes set minus that name", () => {
  const result = toggleGrantedAgent({ scope: "all" }, SECOND, ROSTER);
  assert.equal(result.leftAll, true);
  assert.deepEqual(result.subject, { scope: "set", grantees: [FIRST] });
  assert.equal(agentIsGranted(result.subject, FIRST), true);
  assert.equal(agentIsGranted(result.subject, SECOND), false);
  assert.equal(LEFT_ALL_NOTICE.includes("Future agents"), true);
});

test("unknown grant version does not throw in render", () => {
  const subjects = grantSubjects(
    /** @type {never} */ ({
      version: 3,
      auditChannelId: "7c07e659-3610-42f4-9a5e-1e9973c09da9",
      capabilities: {},
    }),
  );
  assert.equal(subjectsAreEmpty(subjects), true);
});

test("grant-all is one subject map covering five all scopes", () => {
  const subjects = allSubjects();
  for (const name of [
    "agent_config",
    "agent_creation",
    "env_vars",
    "global_config",
    "teams",
  ]) {
    assert.equal(subjects[name].scope, "all", name);
  }
  assert.equal(subjectsAreEmpty(emptySubjects()), true);
  assert.equal(subjectsAreEmpty(subjects), false);
});
