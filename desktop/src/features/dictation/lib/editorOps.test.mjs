import assert from "node:assert/strict";
import test from "node:test";

import { getSchema } from "@tiptap/core";
import { history, undo } from "@tiptap/pm/history";
import { EditorState, TextSelection } from "@tiptap/pm/state";
import StarterKit from "@tiptap/starter-kit";

import {
  insertAtSelection,
  replaceRange,
  suggestionToPickerQuery,
  textToFragment,
} from "./editorOps.ts";

const schema = getSchema([
  StarterKit.configure({ heading: false, trailingNode: false, link: false }),
]);

function makeView(text, cursor) {
  const doc = schema.nodes.doc.create(null, [
    schema.nodes.paragraph.create(null, text ? [schema.text(text)] : []),
  ]);
  let state = EditorState.create({ doc, plugins: [history()] });
  if (cursor !== undefined) {
    state = state.apply(
      state.tr.setSelection(TextSelection.create(state.doc, cursor + 1)),
    );
  }
  const view = {
    state,
    dispatch(tr) {
      view.state = view.state.apply(tr);
    },
  };
  return view;
}

const plain = (view) =>
  view.state.doc.textBetween(0, view.state.doc.content.size, "\n", "⏎");

test("raw words insert at the caret with a separating space", () => {
  const view = makeView("Hello", 5);
  const range = insertAtSelection(view, "anvil can you review");
  assert.equal(plain(view), "Hello anvil can you review");
  assert.equal(
    view.state.doc.textBetween(range.from, range.to),
    "anvil can you review",
  );
});

test("no extra space at the start of an empty composer", () => {
  const view = makeView("", 0);
  insertAtSelection(view, "hi there");
  assert.equal(plain(view), "hi there");
});

test("undo right after the swap restores exactly the raw words", () => {
  const view = makeView("Note: ", 6);
  const range = insertAtSelection(
    view,
    "um anvil can you uh review the the patch",
  );
  replaceRange(view, range.from, range.to, "@Anvil, can you review the patch?");
  assert.equal(plain(view), "Note: @Anvil, can you review the patch?");
  undo(view.state, view.dispatch);
  assert.equal(plain(view), "Note: um anvil can you uh review the the patch");
  undo(view.state, view.dispatch);
  assert.equal(plain(view), "Note: ", "and once more, the original text");
});

test("line breaks become hard breaks of the same length", () => {
  const view = makeView("", 0);
  const range = insertAtSelection(view, "raw");
  replaceRange(view, range.from, range.to, "First.\n\nSecond.");
  assert.equal(plain(view), "First.⏎⏎Second.");
  const fragment = textToFragment(schema, "a\nb");
  assert.equal(fragment.size, "a\nb".length, "positions map 1:1 onto the text");
});

test("an out-of-range swap is refused", () => {
  const view = makeView("short", 5);
  assert.equal(replaceRange(view, 1, 999, "x"), null);
  assert.equal(replaceRange(view, 3, 3, "x"), null);
});

test("a suggestion becomes an @ query for the picker", () => {
  const view = makeView("Hey Theron, hi", 0);
  suggestionToPickerQuery(view, 5, 11);
  assert.equal(plain(view), "Hey @Theron, hi");
  assert.equal(view.state.selection.from, 12, "caret sits right after @Theron");
});

test("inserted text turns line breaks into hard breaks with matching offsets", () => {
  const view = makeView("Hi", 2);
  const range = insertAtSelection(view, "one\ntwo");
  assert.equal(plain(view), "Hi one⏎two");
  // One position per character, including the break.
  assert.equal(range.to - range.from, "one\ntwo".length);
  assert.equal(
    view.state.doc.textBetween(range.from, range.to, "\n", "\n"),
    "one\ntwo",
  );
  assert.equal(
    view.state.selection.from,
    range.to,
    "caret lands after the words",
  );
});
