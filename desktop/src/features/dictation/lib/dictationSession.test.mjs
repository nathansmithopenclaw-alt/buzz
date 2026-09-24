import assert from "node:assert/strict";
import test from "node:test";

import {
  backendIdle,
  claimRecorder,
  registerDictationTarget,
  releaseRecorder,
  resetDictationState,
  routeHotkeyAction,
  trackBackendRelease,
} from "./dictationSession.ts";

function fakeTarget(overrides = {}) {
  const calls = [];
  let status = overrides.status ?? "idle";
  const target = {
    isMain: overrides.isMain ?? false,
    isFocused: () => overrides.focused ?? false,
    focusedAt: () => overrides.focusedAt ?? 0,
    status: () => status,
    start: (mode) => calls.push(`start:${mode}`),
    stop: () => calls.push("stop"),
    cancel: () => calls.push("cancel"),
    discard: () => calls.push("discard"),
  };
  const { id, unregister } = registerDictationTarget(target);
  return {
    calls,
    id,
    unregister,
    setStatus: (next) => {
      status = next;
    },
  };
}

test("a typing-chord cancel never aborts a composer that is still cleaning up", () => {
  const cleaning = fakeTarget({ status: "cleaning", focused: true });
  try {
    routeHotkeyAction("cancel");
    assert.deepEqual(cleaning.calls, []);
  } finally {
    cleaning.unregister();
  }
});

test("a typing-chord cancel stops the composer that is capturing", () => {
  const focused = fakeTarget({ status: "idle", focused: true });
  const recording = fakeTarget({ status: "recording" });
  try {
    routeHotkeyAction("cancel");
    assert.deepEqual(recording.calls, ["cancel"]);
    assert.deepEqual(focused.calls, []);
  } finally {
    focused.unregister();
    recording.unregister();
  }
});

test("tap toggles the focused composer; a hold drives the capturing one", () => {
  const focused = fakeTarget({ focused: true });
  try {
    routeHotkeyAction("toggle");
    assert.deepEqual(focused.calls, ["start:toggle"]);
    focused.setStatus("recording");
    routeHotkeyAction("toggle");
    routeHotkeyAction("stop-hold");
    assert.deepEqual(focused.calls, ["start:toggle", "stop", "stop"]);
  } finally {
    focused.unregister();
  }
});

test("a community reset discards every composer (held results included)", () => {
  const a = fakeTarget();
  const b = fakeTarget({ status: "cleaning" });
  try {
    assert.equal(claimRecorder(a.id), true);
    resetDictationState();
    assert.deepEqual(a.calls, ["discard"]);
    assert.deepEqual(b.calls, ["discard"]);
    assert.equal(claimRecorder(b.id), true, "the recorder slot is free again");
    releaseRecorder(b.id);
  } finally {
    a.unregister();
    b.unregister();
  }
});

test("backendIdle waits for every tracked release, and survives failures", async () => {
  let finishSlow;
  const order = [];
  trackBackendRelease(
    new Promise((resolve) => {
      finishSlow = () => {
        order.push("slow released");
        resolve();
      };
    }),
  );
  trackBackendRelease(Promise.reject(new Error("cancel failed")));
  const waited = backendIdle().then(() => order.push("start"));
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.deepEqual(order, [], "a start waits while a release is in flight");
  finishSlow();
  await waited;
  assert.deepEqual(order, ["slow released", "start"]);
});
