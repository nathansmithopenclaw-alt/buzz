import assert from "node:assert/strict";
import test from "node:test";

const storage = new Map();
globalThis.localStorage = {
  getItem: (key) => storage.get(key) ?? null,
  setItem: (key, value) => storage.set(key, value),
};

const preference = await import("./projectHomePanelPreference.ts");

test("project home panels open automatically by default", () => {
  assert.equal(preference.getOpenProjectHomePanelAutomatically(), true);
});

test("project home panel preference persists explicit opt-out and opt-in", () => {
  preference.setOpenProjectHomePanelAutomatically(false);
  assert.equal(preference.getOpenProjectHomePanelAutomatically(), false);
  assert.equal(
    storage.get("buzz.projects.openHomePanelAutomatically"),
    "false",
  );

  preference.setOpenProjectHomePanelAutomatically(true);
  assert.equal(preference.getOpenProjectHomePanelAutomatically(), true);
  assert.equal(storage.get("buzz.projects.openHomePanelAutomatically"), "true");
});
