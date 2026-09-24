import assert from "node:assert/strict";
import test from "node:test";

import {
  DictationHotkeyMachine,
  HOLD_THRESHOLD_MS,
} from "./dictationHotkey.ts";

/** Deterministic timers: advance(ms) fires due callbacks. */
function fakeTimers() {
  let now = 0;
  let pending = [];
  return {
    timers: {
      now: () => now,
      set: (fn, ms) => {
        const handle = { fn, at: now + ms };
        pending.push(handle);
        return handle;
      },
      clear: (handle) => {
        pending = pending.filter((h) => h !== handle);
      },
    },
    advance(ms) {
      now += ms;
      for (const handle of pending.filter((h) => h.at <= now)) {
        pending = pending.filter((h) => h !== handle);
        handle.fn();
      }
    },
  };
}

function setup(code = "AltRight") {
  const clock = fakeTimers();
  const actions = [];
  const machine = new DictationHotkeyMachine(
    () => code,
    (a) => actions.push(a),
    clock.timers,
  );
  return { machine, actions, clock };
}

const key = (code, extra = {}) => ({ code, ...extra });

test("a quick tap toggles hands-free dictation", () => {
  const { machine, actions, clock } = setup();
  machine.keydown(key("AltRight"));
  clock.advance(120);
  assert.equal(machine.keyup(key("AltRight")), true, "the release is consumed");
  assert.deepEqual(actions, ["toggle"]);
});

test("holding past the threshold is push-to-talk; release stops", () => {
  const { machine, actions, clock } = setup();
  machine.keydown(key("AltRight"));
  clock.advance(HOLD_THRESHOLD_MS - 1);
  assert.deepEqual(actions, [], "the mic never opens before the threshold");
  clock.advance(1);
  assert.deepEqual(actions, ["start-hold"]);
  clock.advance(2000);
  machine.keyup(key("AltRight"));
  assert.deepEqual(actions, ["start-hold", "stop-hold"]);
});

test("Option+letter typing never opens the mic", () => {
  const { machine, actions, clock } = setup();
  machine.keydown(key("AltRight"));
  clock.advance(80);
  machine.keydown(key("KeyE"));
  clock.advance(500);
  assert.equal(
    machine.keyup(key("AltRight")),
    false,
    "the release passes through",
  );
  assert.deepEqual(actions, []);
});

test("another key after the mic opened cancels the dictation", () => {
  const { machine, actions, clock } = setup();
  machine.keydown(key("AltRight"));
  clock.advance(HOLD_THRESHOLD_MS + 50);
  machine.keydown(key("KeyQ"));
  machine.keyup(key("AltRight"));
  assert.deepEqual(actions, ["start-hold", "cancel"]);
});

test("AltGr (Ctrl+AltRight on Windows) is ignored", () => {
  const { machine, actions, clock } = setup();
  machine.keydown(key("AltRight", { ctrlKey: true }));
  clock.advance(1000);
  machine.keyup(key("AltRight", { ctrlKey: true }));
  assert.deepEqual(actions, []);
});

test("auto-repeat does not restart the hold", () => {
  const { machine, actions, clock } = setup();
  machine.keydown(key("AltRight"));
  clock.advance(200);
  machine.keydown(key("AltRight", { repeat: true }));
  clock.advance(200);
  assert.deepEqual(
    actions,
    ["start-hold"],
    "the original press still owns the hold",
  );
});

test("losing focus mid-hold stops, and drops a pending tap", () => {
  const held = setup();
  held.machine.keydown(key("AltRight"));
  held.clock.advance(HOLD_THRESHOLD_MS);
  held.machine.blur();
  assert.deepEqual(held.actions, ["start-hold", "stop-hold"]);

  const tapped = setup();
  tapped.machine.keydown(key("AltRight"));
  tapped.machine.blur();
  tapped.clock.advance(1000);
  assert.deepEqual(tapped.actions, []);
});

test("a custom key works the same way; other keys are ignored", () => {
  const { machine, actions, clock } = setup("F5");
  machine.keydown(key("AltRight"));
  machine.keyup(key("AltRight"));
  machine.keydown(key("F5"));
  clock.advance(50);
  machine.keyup(key("F5"));
  assert.deepEqual(actions, ["toggle"]);
});
