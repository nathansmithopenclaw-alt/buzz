import assert from "node:assert/strict";
import { afterEach, test, mock } from "node:test";
import { relayClient } from "@/shared/api/relayClient.ts";
import { handleRelayClosed } from "@/shared/api/relayClosedRecovery.ts";
import {
  _testRegisterKnownAgents,
  ensureRelayObserverSubscription,
  getAgentObserverSnapshot,
  resetAgentObserverStore,
  subscribeAgentAvatarApplyRequests,
} from "./observerRelayStore.ts";

const OWNER = "a".repeat(64);
const SOURCE = "b".repeat(64);
const originalWindow = globalThis.window;

afterEach(() => {
  resetAgentObserverStore();
  mock.restoreAll();
  globalThis.window = originalWindow;
});

async function settle() {
  for (let i = 0; i < 40; i++) await Promise.resolve();
}

function harness({
  failStarts = 0,
  closeDuringStart = false,
  deferred = false,
} = {}) {
  resetAgentObserverStore();
  const timers = new Map();
  const calls = [];
  const reconnects = new Set();
  const applies = [];
  let nextTimer = 0;
  let identityCalls = 0;
  globalThis.window = {
    setTimeout(fn, ms) {
      const id = ++nextTimer;
      timers.set(id, { fn, ms });
      return id;
    },
    clearTimeout(id) {
      timers.delete(id);
    },
    __TAURI_INTERNALS__: {
      async invoke(command, args) {
        if (command === "get_identity") {
          identityCalls++;
          return { pubkey: OWNER, display_name: "Owner" };
        }
        if (command === "decrypt_observer_event")
          return JSON.parse(JSON.parse(args.eventJson).content);
        throw new Error(`Unexpected IPC: ${command}`);
      },
    },
  };
  mock.method(relayClient, "subscribeToReconnects", (fn) => {
    reconnects.add(fn);
    return () => reconnects.delete(fn);
  });
  mock.method(
    relayClient,
    "subscribeInteractive",
    async (filter, onEvent, options) => {
      const call = { filter, onEvent, options, disposed: false };
      calls.push(call);
      if (calls.length <= failStarts)
        throw new Error("relay temporarily unavailable");
      if (closeDuringStart) close(call);
      if (deferred)
        await new Promise((resolve) => {
          call.resolve = resolve;
        });
      return async () => {
        call.disposed = true;
      };
    },
  );
  function close(call = calls.at(-1)) {
    const subscriptions = new Map([
      [
        "observer",
        {
          mode: "live",
          filter: call.filter,
          onEvent: call.onEvent,
          onTerminalClosed: call.options?.onTerminalClosed,
        },
      ],
    ]);
    handleRelayClosed({
      subscriptions,
      subId: "observer",
      message: "restricted: stale authorization",
      sendReq: async () => {},
    });
    assert.equal(subscriptions.size, 0);
  }
  subscribeAgentAvatarApplyRequests((source, request) =>
    applies.push({ source, request }),
  );
  _testRegisterKnownAgents("app", [SOURCE]);
  return {
    calls,
    timers,
    reconnects,
    applies,
    close,
    identityCalls: () => identityCalls,
    tick() {
      const [id, timer] = timers.entries().next().value;
      timers.delete(id);
      timer.fn();
      return timer.ms;
    },
    reconnect() {
      for (const fn of reconnects) fn();
    },
    emit(call = calls.at(-1), source = SOURCE, sender = source) {
      call.onEvent({
        pubkey: sender,
        tags: [
          ["agent", source],
          ["frame", "telemetry"],
        ],
        content: JSON.stringify({
          seq: 0,
          timestamp: "2026-09-07T12:00:00Z",
          kind: "agent_avatar_apply_request",
          channelId: null,
          sessionId: null,
          payload: {
            type: "agent_avatar_apply_request",
            requestId: "avatar-test",
            authorizationEvent: "signed-owner-authorization",
          },
        }),
      });
    },
  };
}

test("production observer path recovers a terminal CLOSED and delivers the avatar request once", async () => {
  const h = harness();
  await ensureRelayObserverSubscription();
  assert.deepEqual(h.calls[0].filter["#p"], [OWNER]);
  assert.deepEqual(h.calls[0].filter.kinds, [24200]);
  h.close();
  assert.equal(getAgentObserverSnapshot(SOURCE).connectionState, "error");
  assert.equal(h.timers.size, 1);
  assert.equal(h.tick(), 1000);
  await settle();
  assert.equal(h.calls.length, 2);
  assert.equal(getAgentObserverSnapshot(SOURCE).connectionState, "open");
  h.emit(h.calls[0]); // old subscription must not execute a command
  h.emit();
  h.emit(); // replay dedup still applies
  await settle();
  assert.deepEqual(h.applies, [
    {
      source: SOURCE,
      request: {
        type: "agent_avatar_apply_request",
        requestId: "avatar-test",
        authorizationEvent: "signed-owner-authorization",
      },
    },
  ]);
});

test("failed initial subscription retries without remounting app listeners", async () => {
  const h = harness({ failStarts: 1 });
  await ensureRelayObserverSubscription();
  assert.equal(h.timers.size, 1);
  h.tick();
  await settle();
  h.emit();
  await settle();
  assert.equal(h.applies.length, 1);
  assert.equal(h.identityCalls(), 2);
});

test("CLOSED before readiness never caches a dead handle and backs off to 30 seconds", async () => {
  const h = harness({ closeDuringStart: true });
  await ensureRelayObserverSubscription();
  const delays = [];
  for (let i = 0; i < 8; i++) {
    assert.equal(getAgentObserverSnapshot(SOURCE).connectionState, "error");
    assert.equal(h.calls.at(-1).disposed, true);
    delays.push(h.tick());
    await settle();
  }
  assert.deepEqual(
    delays,
    [1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000],
  );
  assert.equal(h.timers.size, 1);
});

test("reconnect promptly restores a missing subscription but leaves a healthy one alone", async () => {
  const h = harness();
  await ensureRelayObserverSubscription();
  h.reconnect();
  await settle();
  assert.equal(h.calls.length, 1);
  h.close();
  h.reconnect();
  await settle();
  assert.equal(h.calls.length, 2);
  assert.equal(h.timers.size, 0);
  h.emit();
  await settle();
  assert.equal(h.applies.length, 1);
});

test("community reset cancels retries and ignores callbacks from the previous owner scope", async () => {
  const h = harness();
  await ensureRelayObserverSubscription();
  h.close();
  resetAgentObserverStore();
  assert.equal(h.timers.size, 0);
  assert.equal(h.reconnects.size, 0);
  h.reconnect();
  h.emit();
  await settle();
  assert.equal(h.calls.length, 1);
  assert.equal(h.applies.length, 0);
});

test("community reset disposes an in-flight subscription when it resolves", async () => {
  const h = harness({ deferred: true });
  await settle();
  assert.equal(h.calls.length, 1);
  resetAgentObserverStore();
  h.calls[0].resolve();
  await settle();
  assert.equal(h.calls[0].disposed, true);
  assert.equal(h.timers.size, 0);
  assert.equal(getAgentObserverSnapshot(SOURCE).connectionState, "idle");
});

test("recovered subscription still rejects unknown agents and mismatched senders", async () => {
  const h = harness();
  await ensureRelayObserverSubscription();
  h.close();
  h.tick();
  await settle();
  h.emit(h.calls.at(-1), "c".repeat(64));
  h.emit(h.calls.at(-1), SOURCE, "c".repeat(64));
  await settle();
  assert.equal(h.applies.length, 0);
  h.emit();
  await settle();
  assert.equal(h.applies.length, 1);
});
