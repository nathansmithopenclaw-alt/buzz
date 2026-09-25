import assert from "node:assert/strict";
import test from "node:test";

import {
  jaroWinkler,
  metaphone,
  NAME_MATCH_THRESHOLD,
  nameSimilarity,
} from "./phonetic.ts";

// Reference values from jellyfish (the dictation benchmark's matcher), so the
// in-app port scores names exactly the way the measured matcher did.
const METAPHONE_REFERENCE = {
  Anvil: "ANFL",
  Beacon: "BKN",
  Bolt: "BLT",
  Codex: "KTKS",
  Ember: "EMBR",
  Fizz: "FS",
  Folio: "FL",
  Gizmo: "JSM",
  Grok: "KRK",
  Helm: "HLM",
  Honey: "HN",
  Keel: "KL",
  Ledger: "LJR",
  Lumen: "LMN",
  Morty: "MRT",
  Nectar: "NKTR",
  Nemo: "NM",
  Patch: "PX",
  Pollen: "PLN",
  Proof: "PRF",
  Quill: "KL",
  Qwen: "KWN",
  Ridge: "RJ",
  Sable: "SBL",
  Theron: "0RN",
  Vela: "FL",
  Vesper: "FSPR",
  Wren: "RN",
  Nathan: "N0N",
  Quen: "KN",
  Gwen: "KWN",
  Theeron: "0RN",
  Faron: "FRN",
  Vella: "FL",
  Ren: "RN",
  Vespa: "FSP",
  Knight: "NT",
  Thomas: "0MS",
  Schema: "SXM",
  Science: "SSNS",
  Judge: "JJ",
  Ghost: "KHST",
  Nation: "NXN",
  Phone: "FN",
  Whistle: "WSTL",
  Xavier: "SFR",
  Laugh: "LKH",
  Sign: "S",
  Campaign: "KMP",
  Signed: "SKNT",
  Bomb: "BM",
  Plumb: "PLM",
  Gnome: "NM",
  Night: "NT",
  Tough: "TKH",
  Hugh: "HKH",
  Aghast: "AKHST",
  Scene: "SSN",
  Chris: "XRS",
  Czech: "KSX",
  Xhosa: "XHS",
  Wheel: "WL",
  Which: "WX",
  Yellow: "YL",
  Yay: "Y",
  Scissors: "SSSRS",
  Edge: "EJ",
};

test("metaphone matches the benchmark's reference implementation", () => {
  for (const [word, key] of Object.entries(METAPHONE_REFERENCE)) {
    assert.equal(metaphone(word), key, word);
  }
  assert.equal(metaphone(""), "");
  assert.equal(metaphone("Bolt's"), "BLTS");
});

test("jaro-winkler matches reference scores", () => {
  const reference = [
    ["quen", "qwen", 0.85],
    ["gwen", "qwen", 0.833333],
    ["theeron", "theron", 0.966667],
    ["faron", "theron", 0.7],
    ["vella", "vela", 0.953333],
    ["vespa", "vesper", 0.893333],
    ["lupin", "lumen", 0.786667],
    ["heel", "keel", 0.833333],
    ["bold", "bolt", 0.883333],
  ];
  for (const [a, b, score] of reference) {
    assert.ok(Math.abs(jaroWinkler(a, b) - score) < 1e-5, `${a}/${b}`);
  }
  assert.equal(jaroWinkler("anvil", "anvil"), 1);
  assert.equal(jaroWinkler("", "anvil"), 0);
});

test("name similarity recovers the benchmark's typical mishearings", () => {
  for (const [spoken, name] of [
    ["Theeron", "Theron"],
    ["Vella", "Vela"],
    ["Vespa", "Vesper"],
    ["Ren", "Wren"],
    ["Bold", "Bolt"],
  ]) {
    assert.ok(
      nameSimilarity(spoken, name) >= NAME_MATCH_THRESHOLD,
      `${spoken} → ${name}`,
    );
  }
  for (const [spoken, name] of [
    ["Lupin", "Lumen"],
    ["Faron", "Theron"],
    ["patch", "Proof"],
  ]) {
    assert.ok(
      nameSimilarity(spoken, name) < NAME_MATCH_THRESHOLD,
      `${spoken} ≠ ${name}`,
    );
  }
  assert.equal(nameSimilarity("anvil", "Anvil"), 1);
});
