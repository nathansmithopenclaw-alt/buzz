import assert from "node:assert/strict";
import test from "node:test";
import { publishBriefConfig } from "./summaryProtocol.ts";
import { syncSummarySchedule } from "./summarySchedule.ts";
import { relayClient } from "@/shared/api/relayClient";
const owner = "11".repeat(32),
  reporter = "22".repeat(32);
const input = {
  ownerPubkey: owner,
  reporterPubkey: reporter,
  reporterName: "Nectar",
  hour: 9,
  minute: 0,
  timezone: "UTC",
  autoInclude: false,
  conversations: [
    { id: "private-source", name: "Grok", kind: "dm", archived: false },
  ],
  sourceConfig: {
    autoInclude: false,
    include: ["private-source"],
    exclude: [],
  },
};
function install(invoke) {
  const previous = globalThis.window;
  globalThis.window = { __TAURI_INTERNALS__: { invoke } };
  return () => {
    globalThis.window = previous;
  };
}
test("DM-only Summary saves an explicit grant, retains the owner tag, and schedules privately without source memberships", async () => {
  const calls = [];
  let signed, published;
  const restore = install(async (command, args) => {
    calls.push([command, args]);
    if (command === "open_dm")
      return {
        id: "reporter-dm",
        name: "DM",
        channel_type: "dm",
        participants: [],
        participant_pubkeys: [],
      };
    if (command === "get_channel_workflows") return [];
    if (command === "create_workflow")
      return {
        id: "schedule",
        revision: "r",
        name: "Summary",
        owner_pubkey: owner,
        channel_id: "reporter-dm",
        definition: {},
        status: "active",
        created_at: 1,
        updated_at: 1,
      };
    if (command === "sign_event") {
      signed = args;
      return JSON.stringify({
        id: "signed-config",
        kind: args.kind,
        content: args.content,
        tags: args.tags,
      });
    }
    throw new Error(`unexpected write/read: ${command}`);
  });
  const oldPublish = relayClient.publishEvent;
  relayClient.publishEvent = async (event) => {
    published = event;
  };
  try {
    await publishBriefConfig(input);
    assert.equal(signed.allowSelfTagging, true);
    assert.ok(signed.tags.some((t) => t[0] === "p" && t[1] === owner));
    assert.equal(JSON.parse(published.content).read_delegation, true);
    assert.deepEqual(JSON.parse(published.content).dms, ["private-source"]);
    assert.equal(
      calls.find((c) => c[0] === "create_workflow")[1].channelId,
      "reporter-dm",
    );
    assert.match(
      calls.find((c) => c[0] === "create_workflow")[1].yamlDefinition,
      /buzz briefs read --owner/,
    );
    assert.deepEqual(calls.find((c) => c[0] === "open_dm")[1].pubkeys, [
      reporter,
    ]);
    assert.equal(
      calls.some((c) => /add.*member|remove.*member/.test(c[0])),
      false,
    );
  } finally {
    restore();
    relayClient.publishEvent = oldPublish;
  }
});
test("schedule deletion failures propagate instead of falsely reporting saved sources", async () => {
  const restore = install(async () => {
    throw new Error("relay unavailable");
  });
  try {
    await assert.rejects(
      syncSummarySchedule({
        ...input,
        previousWorkflowId: "old",
        hostChannelId: null,
      }),
      /relay unavailable/,
    );
  } finally {
    restore();
  }
});
test("schedule lookup failure cannot create a duplicate workflow", async () => {
  const calls = [];
  const restore = install(async (command) => {
    calls.push(command);
    throw new Error("relay unavailable");
  });
  try {
    await assert.rejects(
      syncSummarySchedule({
        ...input,
        previousWorkflowId: "old",
        hostChannelId: "reporter-dm",
      }),
      /relay unavailable/,
    );
    assert.deepEqual(calls, ["get_workflow"]);
  } finally {
    restore();
  }
});
test("manual-only mode removes the old schedule but persists the same read grant", async () => {
  const calls = [];
  let published;
  const restore = install(async (command, args) => {
    calls.push([command, args]);
    if (command === "open_dm")
      return {
        id: "reporter-dm",
        name: "DM",
        channel_type: "dm",
        participants: [],
        participant_pubkeys: [],
      };
    if (command === "delete_workflow") return null;
    if (command === "sign_event")
      return JSON.stringify({
        id: "config",
        kind: args.kind,
        content: args.content,
        tags: args.tags,
      });
    throw new Error(`unexpected ${command}`);
  });
  const oldPublish = relayClient.publishEvent;
  relayClient.publishEvent = async (event) => {
    published = event;
  };
  try {
    await publishBriefConfig({
      ...input,
      schedule: null,
      previousWorkflowId: "old",
    });
    assert.ok(calls.some((x) => x[0] === "delete_workflow"));
    assert.equal(
      calls.some((x) => x[0] === "create_workflow"),
      false,
    );
    assert.equal(JSON.parse(published.content).schedule, null);
    assert.deepEqual(JSON.parse(published.content).dms, ["private-source"]);
    assert.equal(
      published.tags.some((t) => t[0] === "workflow"),
      false,
    );
  } finally {
    restore();
    relayClient.publishEvent = oldPublish;
  }
});
test("rapid schedule saves sign a timestamp after the previous config", async () => {
  let signed;
  const previousCreatedAt = Math.floor(Date.now() / 1000) + 2;
  const restore = install(async (command, args) => {
    if (command === "sign_event") {
      signed = args;
      return JSON.stringify({
        id: "config",
        content: args.content,
        tags: args.tags,
      });
    }
    throw new Error(`unexpected ${command}`);
  });
  const oldPublish = relayClient.publishEvent;
  relayClient.publishEvent = async () => {};
  try {
    await publishBriefConfig({
      ...input,
      sourceConfig: {
        autoInclude: false,
        include: [],
        exclude: ["private-source"],
      },
      schedule: null,
      previousConfigCreatedAt: previousCreatedAt,
    });
    assert.equal(signed.createdAt, previousCreatedAt + 1);
  } finally {
    restore();
    relayClient.publishEvent = oldPublish;
  }
});
