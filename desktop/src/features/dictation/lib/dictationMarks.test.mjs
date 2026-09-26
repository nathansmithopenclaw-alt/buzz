import assert from "node:assert/strict";
import test from "node:test";

import { getSchema } from "@tiptap/core";
import { EditorState, TextSelection } from "@tiptap/pm/state";
import StarterKit from "@tiptap/starter-kit";

import {
  DictationMarksExtension,
  dictationMarksKey,
  MAX_DICTATION_MARKS,
  readDictationMark,
  releaseDictationMark,
  suggestionAtSelection,
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

test("a polish range follows edits made before it", () => {
  const view = makeView("hello raw words here");
  const id = trackDictationMark(view, {
    from: at(6),
    to: at(15),
    kind: "polish",
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
    kind: "polish",
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
    kind: "polish",
  });
  view.dispatch(view.state.tr.insertText("Z", at(5), at(6)));
  assert.equal(readDictationMark(view, id), null);

  const deleted = makeView("aa raw words bb");
  const id2 = trackDictationMark(deleted, {
    from: at(3),
    to: at(12),
    kind: "polish",
  });
  deleted.dispatch(deleted.state.tr.delete(at(0), at(14)));
  assert.equal(readDictationMark(deleted, id2), null);
});

test("release forgets a mark; empty ranges are never tracked", () => {
  const view = makeView("aa raw bb");
  assert.equal(
    trackDictationMark(view, { from: at(3), to: at(3), kind: "polish" }),
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

test("marks render with their classes; polish ranges stay invisible", () => {
  const view = makeView("hey Anvil and raw");
  trackDictationMark(view, {
    from: at(4),
    to: at(9),
    kind: "suggestion",
    entryKey: "p:1",
  });
  trackDictationMark(view, { from: at(14), to: at(17), kind: "polish" });
  const set = plugins[0].props.decorations(view.state);
  const found = set.find();
  const classes = found.map((d) => d.type.attrs.class).sort();
  assert.deepEqual(classes, ["dictation-suggestion"]);
  const suggestion = found.find(
    (d) => d.type.attrs.class === "dictation-suggestion",
  );
  assert.ok(suggestion.type.attrs["data-dictation-suggestion"]);
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

test("typing strictly inside a polish range kills it (the swap must not erase it)", () => {
  const view = makeView("lets meet tomorrow");
  const id = trackDictationMark(view, {
    from: at(0),
    to: at(18),
    kind: "polish",
  });
  // A pure insertion (no deleted text) in the middle of the raw words.
  view.dispatch(view.state.tr.insertText("not ", at(10)));
  assert.equal(readDictationMark(view, id), null);
});

test("a line break typed inside a polish range kills it too", () => {
  const view = makeView("first second");
  const id = trackDictationMark(view, {
    from: at(0),
    to: at(12),
    kind: "polish",
  });
  view.dispatch(
    view.state.tr.replaceWith(at(5), at(5), schema.nodes.hardBreak.create()),
  );
  assert.equal(readDictationMark(view, id), null);
});

test("the caret inside (or at the edge of) a suggestion finds it; elsewhere nothing", () => {
  const view = makeView("ask Ember now");
  const id = trackDictationMark(view, {
    from: at(4),
    to: at(9),
    kind: "suggestion",
    entryKey: "pk:ember",
  });
  const caret = (offset) =>
    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(view.state.doc, at(offset)),
      ),
    );
  caret(6);
  assert.equal(suggestionAtSelection(view.state), id);
  caret(9);
  assert.equal(suggestionAtSelection(view.state), id);
  caret(1);
  assert.equal(suggestionAtSelection(view.state), null);
});

test("Alt+Enter in a suggestion opens it; plain Enter is left to the composer", () => {
  const view = makeView("ask Ember now");
  const id = trackDictationMark(view, {
    from: at(4),
    to: at(9),
    kind: "suggestion",
    entryKey: "pk:ember",
  });
  view.dispatch(
    view.state.tr.setSelection(TextSelection.create(view.state.doc, at(6))),
  );
  const events = [];
  view.dom = { dispatchEvent: (event) => events.push(event) };
  const handleKeyDown = plugins[0].props.handleKeyDown;
  const key = (init) => ({ key: "Enter", isComposing: false, ...init });
  assert.equal(handleKeyDown(view, key({ altKey: false })), false);
  assert.equal(
    handleKeyDown(view, key({ altKey: true, metaKey: true })),
    false,
  );
  assert.equal(handleKeyDown(view, key({ altKey: true })), true);
  assert.equal(events.length, 1);
  assert.equal(events[0].detail.id, id);
});
