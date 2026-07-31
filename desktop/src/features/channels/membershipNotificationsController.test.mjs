import assert from "node:assert/strict";
import test from "node:test";

import { startMembershipNotifications } from "./membershipNotificationsController.ts";
import { handleRelayClosed } from "../../shared/api/relayClosedRecovery.ts";

function makeTimerHost() {
  let now = 0;
  let nextId = 1;
  const timers = new Map();
  return {
    host: {
      setTimeout(handler, delayMs) {
        const id = nextId++;
        timers.set(id, { handler, fireAt: now + delayMs });
        return id;
      },
      clearTimeout(id) {
        timers.delete(id);
      },
    },
    pendingCount() {
      return timers.size;
    },
    tickTo(nextNow) {
      now = nextNow;
      for (const [id, timer] of [...timers.entries()]) {
        if (timer.fireAt <= now) {
          timers.delete(id);
          timer.handler();
        }
      }
    },
  };
}

function makeRelay() {
  const calls = [];
  const reconnectListeners = new Set();
  return {
    calls,
    async subscribeLive(filter, onEvent, options) {
      const call = { disposed: false, filter, onEvent, options };
      calls.push(call);
      return async () => {
        call.disposed = true;
      };
    },
    subscribeToReconnects(listener) {
      reconnectListeners.add(listener);
      return () => reconnectListeners.delete(listener);
    },
    reconnect() {
      for (const listener of reconnectListeners) listener();
    },
    terminalClose(callIndex, message) {
      const call = calls[callIndex];
      const subscriptions = new Map([
        [
          "live-test",
          {
            mode: "live",
            filter: call.filter,
            onEvent: call.onEvent,
            onTerminalClosed: call.options.onTerminalClosed,
          },
        ],
      ]);
      handleRelayClosed({
        subscriptions,
        subId: "live-test",
        message,
        sendReq: () => Promise.resolve(),
      });
      assert.equal(subscriptions.size, 0);
    },
  };
}

async function settleUntil(predicate) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (predicate()) return;
    await Promise.resolve();
  }
  assert.fail("condition did not settle");
}

function event(kind, tags) {
  return {
    id: `event-${kind}`,
    pubkey: "relay",
    created_at: 1_000,
    kind,
    tags,
    content: "",
    sig: "sig",
  };
}

test("44100 invalidates channels and membership detail", async () => {
  const relay = makeRelay();
  const timers = makeTimerHost();
  let channelsInvalidated = 0;
  const membershipInvalidations = [];
  const controller = startMembershipNotifications({
    currentPubkey: "owner",
    invalidateChannels: () => {
      channelsInvalidated += 1;
    },
    isChannelsFetching: () => 0,
    invalidateChannelMembership: (channelId) =>
      membershipInvalidations.push(channelId),
    relay,
    timerHost: timers.host,
    nowSeconds: () => 1_000,
  });
  await controller.ready;

  assert.equal(relay.calls.length, 2);
  assert.deepEqual(relay.calls[0].filter.kinds, [44100, 44101]);
  assert.deepEqual(relay.calls[1].filter, {
    kinds: [39002],
    "#p": ["owner"],
    limit: 50,
    since: 1_000,
  });

  relay.calls[0].onEvent(event(44100, [["h", "channel-new"]]));
  assert.deepEqual(membershipInvalidations, ["channel-new"]);
  assert.equal(channelsInvalidated, 0, "list invalidation is debounced");
  timers.tickTo(250);
  assert.equal(channelsInvalidated, 1);
  controller.stop();
});

test("terminal CLOSED followed by reconnect re-arms both subscriptions", async () => {
  const relay = makeRelay();
  const timers = makeTimerHost();
  let channelsInvalidated = 0;
  const controller = startMembershipNotifications({
    currentPubkey: "owner",
    invalidateChannels: () => {
      channelsInvalidated += 1;
    },
    isChannelsFetching: () => 0,
    invalidateChannelMembership: () => {},
    relay,
    timerHost: timers.host,
    nowSeconds: () => 1_000,
  });
  await controller.ready;
  assert.equal(relay.calls.length, 2);

  relay.terminalClose(0, "restricted: stale authorization");
  assert.equal(timers.pendingCount(), 1, "terminal close schedules a re-arm");
  relay.reconnect();

  await settleUntil(() => relay.calls.length === 4);
  assert.equal(relay.calls[0].disposed, true);
  assert.equal(relay.calls[1].disposed, true);
  assert.deepEqual(relay.calls[2].filter.kinds, [44100, 44101]);
  assert.deepEqual(relay.calls[3].filter.kinds, [39002]);
  timers.tickTo(250);
  assert.equal(
    channelsInvalidated,
    1,
    "reconnect invalidates the channel list",
  );
  controller.stop();
});

test("bursty 39002 membership snapshots coalesce to one channels invalidation", async () => {
  const relay = makeRelay();
  const timers = makeTimerHost();
  let channelsInvalidated = 0;
  const controller = startMembershipNotifications({
    currentPubkey: "owner",
    invalidateChannels: () => {
      channelsInvalidated += 1;
    },
    isChannelsFetching: () => 0,
    invalidateChannelMembership: () => {},
    relay,
    timerHost: timers.host,
    nowSeconds: () => 1_000,
  });
  await controller.ready;

  for (let index = 0; index < 10; index += 1) {
    relay.calls[1].onEvent(event(39002, [["d", `channel-${index}`]]));
  }
  timers.tickTo(249);
  assert.equal(channelsInvalidated, 0);
  timers.tickTo(250);
  assert.equal(channelsInvalidated, 1);
  controller.stop();
});
