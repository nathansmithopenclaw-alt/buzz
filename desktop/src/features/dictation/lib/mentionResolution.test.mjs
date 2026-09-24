import assert from "node:assert/strict";
import test from "node:test";

import { buildDictationRoster, modelRosterNames } from "./dictationRoster.ts";
import {
  findAddressedNames,
  resolveModelOutput,
  resolveVerbatim,
} from "./mentionResolution.ts";
import { applySpokenFormatting } from "./spokenFormatting.ts";

const pk = (n) => n.toString(16).padStart(64, "0");
const SELF = pk(99);

const member = (name, n, extra = {}) => ({
  kind: "identity",
  pubkey: pk(n),
  displayName: name,
  isMember: true,
  isAgent: true,
  ...extra,
});

const AGENTS = [
  "Anvil",
  "Beacon",
  "Bolt",
  "Codex",
  "Codex Desktop",
  "Ember",
  "Fizz",
  "Grok",
  "Helm",
  "Honey",
  "Keel",
  "Ledger",
  "Lumen",
  "Morty",
  "Nectar",
  "Nemo",
  "Patch",
  "Pollen",
  "Proof",
  "Quill",
  "Qwen",
  "Ridge",
  "Sable",
  "Theron",
  "Vela",
  "Vesper",
  "Wren",
];

function roster({ extra = [] } = {}) {
  const candidates = [
    ...AGENTS.map((name, i) => member(name, i + 1)),
    member("Nathan Smith", 90, { isAgent: false }),
    member("Me Myself", 99, { isAgent: false }),
    ...extra,
  ];
  const memberPubkeys = new Set(
    candidates.filter((c) => c.isMember && c.pubkey).map((c) => c.pubkey),
  );
  return buildDictationRoster({
    candidates,
    currentPubkey: SELF,
    memberPubkeys,
  });
}

const summarize = (segments) =>
  segments.map((s) =>
    s.type === "text"
      ? s.text
      : s.type === "mention"
        ? `<@${s.entry.displayName}>`
        : `<?${s.entry.displayName}:${s.text}>`,
  );

const mentionsIn = (segments) =>
  segments.filter((s) => s.type === "mention").map((s) => s.entry.displayName);
const suggestionsIn = (segments) =>
  segments
    .filter((s) => s.type === "suggestion")
    .map((s) => s.entry.displayName);

// ── Verbatim "at" rule (grill Q11 cases) ──────────────────────────────────────

test("verbatim: explicit 'at <name>' becomes a mention; everything else never does", () => {
  const r = roster();
  const cases = [
    ["At patch, please rebase your branch onto main.", ["Patch"]],
    ["Quill, draft the change log, and at ledger, reconcile the invoices.", ["Ledger"]],
    ["At Helm, can you check the deploy?", ["Helm"]],
    ["at helm, can you check the deploy", ["Helm"]],
    ["At Theron please review this", ["Theron"]],
    ["Ship it, and at Anvil, please verify.", ["Anvil"]],
    ["Look at Helm's comment on the PR.", []],
    ["We're at patch three of the rollout.", []],
    ["At ledger level the numbers don't match.", []],
    ["Let's meet at noon, Helm.", []],
    ["Stitch is at the helm today.", []],
  ];
  for (const [text, expected] of cases) {
    assert.deepEqual(mentionsIn(resolveVerbatim(text, r)), expected, text);
  }
});

test("verbatim: the spoken 'at' is consumed, the rest of the sentence kept", () => {
  assert.deepEqual(summarize(resolveVerbatim("At Helm, can you check that?", roster())), [
    "<@Helm>",
    ", can you check that?",
  ]);
  assert.deepEqual(
    summarize(resolveVerbatim("Look at that. And at Anvil, verify.", roster())),
    ["Look at that. And ", "<@Anvil>", ", verify."],
  );
});

test("verbatim: other addressed names are suggestions, never mentions", () => {
  const out = resolveVerbatim("Anvil, can you review the patch Codex sent?", roster());
  assert.deepEqual(mentionsIn(out), []);
  assert.deepEqual(suggestionsIn(out), ["Anvil"]);
});

// ── Addressed vs referenced (benchmark cases) ─────────────────────────────────

test("address detection follows the benchmark's addressed/referenced labels", () => {
  const r = roster();
  const names = (text) => findAddressedNames(text, r).map((h) => h.entry.displayName);
  const cases = [
    ["Anvil, can you review the patch Codex sent this morning?", ["Anvil"]],
    ["Ask Helm to check whether Titus finished the deploy.", ["Helm"]],
    ["Can you patch the login bug before the demo?", []],
    ["Theron and Vela, I need both of you on the release notes.", ["Theron", "Vela"]],
    ["Bolt's last run failed, so let's hold off.", []],
    ["Hey Qwen, summarize the thread for Nathan.", ["Qwen"]],
    ["I think we should move the demo to Friday, Wren.", ["Wren"]],
    ["The ledger doesn't balance and the keel of the boat is cracked.", []],
    ["Grok, what does Sable think about the pricing?", ["Grok"]],
    ["Vesper, loop in Morty and Nemo on this one.", ["Vesper", "Morty", "Nemo"]],
    ["Proof, check the proof of concept Fizz wrote.", ["Proof"]],
    ["Honey, Pollen and Nectar, standup is in five minutes.", ["Honey", "Pollen", "Nectar"]],
    ["Scratch that, tell Ember the build is green.", ["Ember"]],
    ["Keel, Ridge, and Lumen, please review Beacon's design doc.", ["Keel", "Ridge", "Lumen"]],
    ["Helm, add the Game Studio team to this channel.", ["Helm"]],
  ];
  for (const [text, expected] of cases) {
    assert.deepEqual(names(text), expected, text);
  }
});

test("near-spellings resolve through the sound-alike matcher", () => {
  const r = roster();
  const names = (text) => findAddressedNames(text, r).map((h) => h.entry.displayName);
  assert.deepEqual(names("Theeron and Vella, I need both of you."), ["Theron", "Vela"]);
  assert.deepEqual(names("Vespa, loop in Morty on this one."), ["Vesper", "Morty"]);
});

// ── Model output validation ───────────────────────────────────────────────────

test("model markers become mentions only for auto-mentionable roster entries", () => {
  const out = resolveModelOutput("@[Anvil], can you review the patch Codex sent?", roster());
  assert.deepEqual(summarize(out), ["<@Anvil>", ", can you review the patch Codex sent?"]);
});

test("invented or ambiguous names from the model never become mentions", () => {
  const r = roster({ extra: [member("Twin", 60), member("Twin", 61)] });
  const out = resolveModelOutput("@[Zorblax], hi and @[Twin], hi", r);
  assert.deepEqual(summarize(out), ["Zorblax, hi and Twin, hi"]);
  // A misspelled marker can still surface as a click-to-confirm suggestion.
  const near = resolveModelOutput("@[Groc], ship it", r);
  assert.deepEqual(mentionsIn(near), []);
  assert.deepEqual(suggestionsIn(near), ["Grok"]);
});

test("the exact spoken name wins over a longer overlapping name", () => {
  const r = roster();
  assert.deepEqual(mentionsIn(resolveModelOutput("@[Codex] and @[Codex Desktop], sync up", r)), [
    "Codex",
    "Codex Desktop",
  ]);
});

test("personas, non-members and out-of-channel teams are only suggestions", () => {
  const r = roster({
    extra: [
      { kind: "persona", personaId: "p1", displayName: "Scout", isMember: false, isAgent: true },
      member("Outsider", 70, { isMember: false }),
      {
        kind: "team",
        teamId: "t-out",
        displayName: "Game Studio",
        isMember: false,
        isAgent: true,
        teamMembers: [
          { kind: "identity", pubkey: pk(1), displayName: "Anvil" },
          { kind: "identity", pubkey: pk(71), displayName: "Stranger" },
        ],
      },
    ],
  });
  const out = resolveModelOutput("@[Scout], @[Outsider] and @[Game Studio], hello", r);
  assert.deepEqual(mentionsIn(out), []);
  assert.deepEqual(suggestionsIn(out), ["Scout", "Outsider", "Game Studio"]);
});

test("a team whose members are all in the channel is auto-mentionable", () => {
  const r = roster({
    extra: [
      {
        kind: "team",
        teamId: "t-in",
        displayName: "Release Crew",
        isMember: false,
        isAgent: true,
        teamMembers: [
          { kind: "identity", pubkey: pk(1), displayName: "Anvil" },
          { kind: "identity", pubkey: pk(9), displayName: "Helm" },
        ],
      },
    ],
  });
  assert.deepEqual(mentionsIn(resolveModelOutput("@[Release Crew], ship it", r)), [
    "Release Crew",
  ]);
  const withPersona = roster({
    extra: [
      {
        kind: "team",
        teamId: "t-p",
        displayName: "Half Built",
        isMember: false,
        isAgent: true,
        teamMembers: [
          { kind: "identity", pubkey: pk(1), displayName: "Anvil" },
          { kind: "persona", personaId: "p9", displayName: "Draft" },
        ],
      },
    ],
  });
  assert.deepEqual(mentionsIn(resolveModelOutput("@[Half Built], go", withPersona)), []);
});

test("the speaker is never in the roster, so never mentioned", () => {
  const r = roster();
  assert.ok(!r.some((e) => e.pubkey === SELF));
  assert.deepEqual(mentionsIn(resolveModelOutput("@[Me Myself], note to self", r)), []);
});

test("unique first names are aliases; shared first names are not", () => {
  const r = roster({ extra: [member("Nathan Jones", 91, { isAgent: false })] });
  const nathan = r.find((e) => e.displayName === "Nathan Smith");
  assert.deepEqual(nathan.aliases, [], "two members share 'Nathan'");
  const solo = roster().find((e) => e.displayName === "Nathan Smith");
  assert.deepEqual(solo.aliases, ["nathan"]);
  assert.deepEqual(mentionsIn(resolveModelOutput("@[Nathan], hi", roster())), ["Nathan Smith"]);
});

test("addressed names the model left unmarked become suggestions", () => {
  const r = roster({ extra: [member("Outsider", 70, { isMember: false })] });
  const out = resolveModelOutput("@[Anvil], and Outsider, can you both look?", r);
  assert.deepEqual(mentionsIn(out), ["Anvil"]);
  assert.deepEqual(suggestionsIn(out), ["Outsider"]);
});

test("the model roster lists auto-mentionable names only, capped at 150", () => {
  const names = modelRosterNames(roster({ extra: [member("Outsider", 70, { isMember: false })] }));
  assert.ok(names.includes("Anvil"));
  assert.ok(!names.includes("Outsider"));
  assert.ok(!names.includes("Me Myself"));
  const big = buildDictationRoster({
    candidates: Array.from({ length: 300 }, (_, i) => member(`M${i}`, 1000 + i)),
    currentPubkey: null,
    memberPubkeys: new Set(Array.from({ length: 300 }, (_, i) => pk(1000 + i))),
  });
  assert.equal(modelRosterNames(big).length, 150);
});

test("recently mentioned members are listed first", () => {
  const candidates = [member("Anvil", 1), member("Helm", 2), member("Wren", 3)];
  const ordered = buildDictationRoster({
    candidates,
    currentPubkey: null,
    memberPubkeys: new Set([pk(1), pk(2), pk(3)]),
    recentPubkeys: [pk(3)],
  });
  assert.equal(ordered[0].displayName, "Wren");
});

// ── Spoken formatting ─────────────────────────────────────────────────────────

test("spoken 'new line' / 'new paragraph' commands become breaks", () => {
  assert.equal(
    applySpokenFormatting("Ship it today. New paragraph. then tell the team."),
    "Ship it today.\n\nThen tell the team.",
  );
  assert.equal(applySpokenFormatting("First item, new line, second item"), "First item,\nSecond item");
  assert.equal(
    applySpokenFormatting("We need a new line of products this quarter."),
    "We need a new line of products this quarter.",
  );
});
