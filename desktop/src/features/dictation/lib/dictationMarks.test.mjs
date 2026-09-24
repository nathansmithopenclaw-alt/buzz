import assert from "node:assert/strict";
import test from "node:test";

import { getSchema } from "@tiptap/core";
import { EditorState } from "@tiptap/pm/state";
import StarterKit from "@tiptap/starter-kit";

import {
  DictationMarksExtension,
  dictationMarksKey,
  MAX_DICTATION_MARKS,
  readDictationMark,
  releaseDictationMark,
  trackDictationMark,
} from "./dictationMarks.ts";

const schema = getSchema([
  StarterKit.configure({ heading: false, trailingNode: false, link: false }),
]);
const plugins = DictationMarksExtension.config.addProseMirrorPlugins.call({});

/** A minimal view over a real EditorState with the real plugin. */
function makeView(text) {
  const doc = schema.nodes.doc.create(null, [
    schema.nodes.paragraph.create(null, text ? [schema.text(text)] : []),
  ]);
  const view = {
    state: EditorState.create({ doc, plugins }),
    dispatch(tr) {
      view.state = view.state.apply(tr);
    },
  };
  return view;
}

// Paragraph content starts at position 1.
const at = (offset) => offset + 1;

test("a pending range follows edits made before it", () => {
  const view = makeView("hello raw words here");
  const id = trackDictationMark(view, {
    from: at(6),
    to: at(15),
    kind: "pending",
  });
  view.dispatch(view.state.tr.insertText("XX", at(0)));
  const mark = readDictationMark(view, id);
  assert.deepEqual([mark.from, mark.to], [at(8), at(17)]);
  assert.equal(view.state.doc.textBetween(mark.from, mark.to), "raw words");
});

test("typing at either edge stays outside the range", () => {
  const view = makeView("aa raw bb");
  const id = trackDictationMark(view, {
    from: at(3),
    to: at(6),
    kind: "pending",
  });
  view.dispatch(view.state.tr.insertText("<", at(3)));
  const afterStart = readDictationMark(view, id);
  view.dispatch(view.state.tr.insertText(">", afterStart.to));
  const mark = readDictationMark(view, id);
  assert.equal(view.state.doc.textBetween(mark.from, mark.to), "raw");
});

test("any edit inside the range kills it (late cleanup cannot overwrite edits)", () => {
  const view = makeView("aa raw words bb");
  const id = trackDictationMark(view, {
    from: at(3),
    to: at(12),
    kind: "pending",
  });
  view.dispatch(view.state.tr.insertText("Z", at(5), at(6)));
  assert.equal(readDictationMark(view, id), null);

  const deleted = makeView("aa raw words bb");
  const id2 = trackDictationMark(deleted, {
    from: at(3),
    to: at(12),
    kind: "pending",
  });
  deleted.dispatch(deleted.state.tr.delete(at(0), at(14)));
  assert.equal(readDictationMark(deleted, id2), null);
});

test("release forgets a mark; empty ranges are never tracked", () => {
  const view = makeView("aa raw bb");
  assert.equal(
    trackDictationMark(view, { from: at(3), to: at(3), kind: "pending" }),
    null,
  );
  const id = trackDictationMark(view, {
    from: at(3),
    to: at(6),
    kind: "suggestion",
  });
  releaseDictationMark(view, id);
  assert.equal(readDictationMark(view, id), null);
  releaseDictationMark(view, id); // idempotent
});

test("tracking does not create undo history", () => {
  const view = makeView("aa raw bb");
  let sawHistoryFlag = null;
  const dispatch = view.dispatch;
  view.dispatch = (tr) => {
    sawHistoryFlag = tr.getMeta("addToHistory");
    dispatch(tr);
  };
  trackDictationMark(view, { from: at(3), to: at(6), kind: "flash" });
  assert.equal(sawHistoryFlag, false);
});

test("marks render with their classes and suggestions are clickable targets", () => {
  const view = makeView("hey Anvil and raw");
  trackDictationMark(view, {
    from: at(4),
    to: at(9),
    kind: "suggestion",
    entryKey: "p:1",
  });
  trackDictationMark(view, { from: at(14), to: at(17), kind: "pending" });
  const set = plugins[0].props.decorations(view.state);
  const found = set.find();
  const classes = found.map((d) => d.type.attrs.class).sort();
  assert.deepEqual(classes, ["dictation-pending", "dictation-suggestion"]);
  const suggestion = found.find(
    (d) => d.type.attrs.class === "dictation-suggestion",
  );
  assert.ok(suggestion.type.attrs["data-dictation-suggestion"]);
  const pending = found.find((d) => d.type.attrs.class === "dictation-pending");
  assert.equal(pending.type.attrs["aria-busy"], "true");
});

test("the mark set is capped", () => {
  const view = makeView("x".repeat(200));
  for (let i = 0; i < MAX_DICTATION_MARKS + 10; i++) {
    trackDictationMark(view, { from: at(i), to: at(i + 1), kind: "flash" });
  }
  assert.equal(
    dictationMarksKey.getState(view.state).size,
    MAX_DICTATION_MARKS,
  );
});
