import assert from "node:assert/strict";
import test from "node:test";
import { relayClient } from "@/shared/api/relayClient";
import { runSummary } from "./runSummary.ts";
const owner = "11".repeat(32),
  reporter = "22".repeat(32),
  relay = "wss://test.example";
async function fixture(run, options = {}) {
  const calls = [];
  const priorWindow = globalThis.window,
    priorFetch = relayClient.fetchEvents;
  relayClient.fetchEvents = async (filter) => {
    assert.deepEqual(filter.authors, [owner]);
    return [
      {
        content: JSON.stringify({
          v: 1,
          read_delegation: true,
          reporter_pubkey: reporter,
          channels: [],
          dms: options.empty ? [] : ["approved-source"],
          schedule: null,
        }),
      },
    ];
  };
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: async (command, args) => {
        calls.push([command, args]);
        if (command === "list_managed_agents")
          return options.missing
            ? []
            : [
                {
                  pubkey: reporter,
                  name: "Nectar",
                  status: options.running ? "running" : "stopped",
                },
              ];
        if (command === "start_managed_agent") {
          if (options.startError) throw new Error("start failed");
          return { pubkey: reporter, name: "Nectar", status: "running" };
        }
        if (command === "open_dm")
          return {
            id: "reporter-dm",
            name: "Nectar",
            channel_type: "dm",
            participants: [],
            participant_pubkeys: [owner, reporter],
          };
        if (command === "send_channel_message") {
          if (options.sendError) throw new Error("delivery uncertain");
          return { event_id: "request", created_at: 1 };
        }
        throw new Error(`Unexpected side effect ${command}`);
      },
    },
  };
  try {
    await run(calls);
  } finally {
    globalThis.window = priorWindow;
    relayClient.fetchEvents = priorFetch;
  }
}
test("manual-only Summary starts the saved reporter and sends one scoped request without schedule/grant writes", async () => {
  await fixture(async (calls) => {
    const result = await runSummary(owner, relay);
    assert.equal(result.eventId, "request");
    assert.deepEqual(
      calls.map((x) => x[0]),
      [
        "list_managed_agents",
        "start_managed_agent",
        "open_dm",
        "send_channel_message",
      ],
    );
    for (const [, args] of calls.slice(1)) {
      assert.equal(args.expectedRelayUrl, relay);
      assert.equal(args.expectedSignerPubkey, owner);
    }
    const send = calls.at(-1)[1];
    assert.deepEqual(send.mentionPubkeys, [reporter]);
    assert.equal(send.channelId, "reporter-dm");
    assert.match(send.content, /since your previous Summary/);
    assert.match(send.content, /Do not invent work/);
  });
});
test("manual request leaves an already running reporter running", async () => {
  await fixture(
    async (calls) => {
      await runSummary(owner, relay);
      assert.equal(
        calls.some((x) => x[0] === "start_managed_agent"),
        false,
      );
    },
    { running: true },
  );
});
for (const [name, options, message] of [
  ["empty sources", { empty: true }, /at least one/],
  ["missing reporter", { missing: true }, /unavailable/],
  ["startup failure", { startError: true }, /start failed/],
]) {
  test(`manual run rejects ${name} before sending`, async () =>
    fixture(async (calls) => {
      await assert.rejects(runSummary(owner, relay), message);
      assert.equal(
        calls.some((x) => x[0] === "send_channel_message"),
        false,
      );
    }, options));
}
test("uncertain manual delivery propagates and never automatically retries", async () =>
  fixture(
    async (calls) => {
      await assert.rejects(runSummary(owner, relay), /delivery uncertain/);
      assert.equal(
        calls.filter((x) => x[0] === "send_channel_message").length,
        1,
      );
    },
    { sendError: true },
  ));
test("manual request requires owner and relay scope", async () => {
  await assert.rejects(runSummary(owner, ""), /Sign in/);
  await assert.rejects(runSummary("", relay), /Sign in/);
});
