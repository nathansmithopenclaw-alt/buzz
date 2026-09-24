import assert from "node:assert/strict";
import test from "node:test";

import { renderSegments, replaceLastOccurrence } from "./renderSegments.ts";

const pk = (n) => n.toString(16).padStart(64, "0");
const anvil = { key: "p:1", displayName: "Anvil", aliases: [], kind: "identity", pubkey: pk(1), isAgent: true, autoMention: true };
const theron = { key: "p:2", displayName: "Theron", aliases: [], kind: "identity", pubkey: pk(2), isAgent: true, autoMention: false };
const crew = {
  key: "team:t",
  displayName: "Release Crew",
  aliases: [],
  kind: "team",
  teamId: "t",
  isAgent: true,
  autoMention: true,
  teamMembers: [
    { kind: "identity", pubkey: pk(1), displayName: "Anvil" },
    { kind: "identity", pubkey: pk(3), displayName: "Helm" },
  ],
};

/** Registration double that records calls and returns the plain name. */
function recorder(labels = {}) {
  const calls = [];
  const register = (displayName, pubkey, options) => {
    calls.push({ displayName, pubkey, ...options });
    return labels[displayName] ?? displayName;
  };
  return { calls, register };
}

test("mentions use the label their registration returns", () => {
  const { calls, register } = recorder({ Anvil: `Anvil (${pk(1)})` });
  const out = renderSegments(
    [{ type: "mention", entry: anvil }, { type: "text", text: ", can you review?" }],
    register,
  );
  assert.equal(out.text, `@Anvil (${pk(1)}), can you review?`);
  assert.deepEqual(calls, [{ displayName: "Anvil", pubkey: pk(1), isAgent: true }]);
  assert.deepEqual(out.mentionRefs, [{ displayName: `Anvil (${pk(1)})`, pubkey: pk(1), isAgent: true }]);
  assert.deepEqual(out.spans, [{ from: 0, to: out.text.indexOf(","), kind: "mention", entryKey: "p:1" }]);
});

test("suggestions are plain text with a span and no binding", () => {
  const { calls, register } = recorder();
  const out = renderSegments(
    [{ type: "text", text: "Hey " }, { type: "suggestion", entry: theron, text: "Theron" }, { type: "text", text: ", hi" }],
    register,
  );
  assert.equal(out.text, "Hey Theron, hi");
  assert.equal(calls.length, 0);
  assert.deepEqual(out.spans, [{ from: 4, to: 10, kind: "suggestion", entryKey: "p:2" }]);
});

test("teams render as Team(@a @b) with every member bound", () => {
  const { calls, register } = recorder();
  const out = renderSegments([{ type: "mention", entry: crew }, { type: "text", text: ", ship it" }], register);
  assert.equal(out.text, "Release Crew(@Anvil @Helm), ship it");
  assert.equal(calls.length, 2);
  assert.deepEqual(out.mentionRefs.map((r) => r.displayName), ["Anvil", "Helm"]);
});

test("a trailing mention gets a separating space", () => {
  const { register } = recorder();
  const out = renderSegments([{ type: "text", text: "Thanks " }, { type: "mention", entry: anvil }], register);
  assert.equal(out.text, "Thanks @Anvil ");
});

test("a failed registration falls back to the plain name", () => {
  const out = renderSegments([{ type: "mention", entry: anvil }], () => undefined);
  assert.equal(out.text, "Anvil ");
  assert.deepEqual(out.mentionRefs, []);
  assert.deepEqual(out.spans, []);
});

test("draft delivery replaces the last occurrence only", () => {
  assert.equal(replaceLastOccurrence("hi raw x raw", "raw", "NEW"), "hi raw x NEW");
  assert.equal(replaceLastOccurrence("nothing here", "raw", "NEW"), null);
  assert.equal(replaceLastOccurrence("abc", "", "NEW"), null);
});
