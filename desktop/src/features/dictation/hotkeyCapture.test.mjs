import { test } from "node:test";
import assert from "node:assert/strict";
import {
  claimDictationKey,
  isCapturingDictationKey,
  KEY_CAPTURE_EVENT,
  isAltGraphEvent,
  isAvailableDictationKey,
} from "./hotkeyCapture.ts";

test("AltGr is ignored on Windows/Linux but macOS Option remains available", () => {
  for (const event of [
    {
      code: "AltRight",
      key: "AltGraph",
      ctrlKey: false,
      altKey: true,
      getModifierState: () => false,
    },
    {
      code: "AltRight",
      key: "Alt",
      ctrlKey: false,
      altKey: true,
      getModifierState: () => true,
    },
    {
      code: "AltRight",
      key: "Alt",
      ctrlKey: true,
      altKey: true,
      getModifierState: () => false,
    },
  ]) {
    assert.equal(isAltGraphEvent(event, false), true);
    assert.equal(isAltGraphEvent(event, true), false);
  }
});

test("key validation reserves typing, navigation and OS commands", () => {
  for (const code of [
    "KeyA",
    "Digit1",
    "Enter",
    "Space",
    "Escape",
    "Tab",
    "ControlLeft",
    "MetaRight",
    "F5",
    "F11",
    "F12",
    "F013",
    "AltGraph",
  ])
    assert.equal(isAvailableDictationKey(code), false, code);
  for (const code of ["AltLeft", "AltRight", "F8", "F9", "F13", "F24"])
    assert.equal(isAvailableDictationKey(code), true, code);
});

test("capture ownership suspends shortcuts until every recorder releases", () => {
  globalThis.window = new EventTarget();
  let notifications = 0;
  window.addEventListener(KEY_CAPTURE_EVENT, () => notifications++);
  const first = claimDictationKey();
  const second = claimDictationKey();
  assert.equal(isCapturingDictationKey(), true);
  assert.equal(notifications, 2);
  first();
  assert.equal(isCapturingDictationKey(), true);
  second();
  assert.equal(isCapturingDictationKey(), false);
  delete globalThis.window;
});
