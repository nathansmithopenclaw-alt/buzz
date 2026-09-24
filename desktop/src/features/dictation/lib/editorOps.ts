import { closeHistory } from "@tiptap/pm/history";
import { Fragment, type Schema } from "@tiptap/pm/model";

import type { DictationMarksView } from "./dictationMarks";

/**
 * Composer edits dictation makes. Each is its own undo step: inserting the
 * raw words, and later swapping them for the cleaned text, so ⌘Z right after
 * the swap brings back exactly what was spoken.
 */

/** Text with "\n" line breaks as inline content (hard breaks between runs). */
export function textToFragment(schema: Schema, text: string): Fragment {
  const hardBreak = schema.nodes.hardBreak;
  const nodes = [];
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    if (lines[i]) nodes.push(schema.text(lines[i]));
    if (i < lines.length - 1 && hardBreak) nodes.push(hardBreak.create());
  }
  return Fragment.fromArray(nodes);
}

/**
 * Inserts dictated words at the selection (replacing any selected text) as
 * one undo step, adding a separating space after a word. Returns the range
 * the words occupy.
 */
export function insertAtSelection(
  view: DictationMarksView,
  text: string,
): { from: number; to: number } | null {
  if (view.isDestroyed || !text) return null;
  const { from, to } = view.state.selection;
  const before = from > 1 ? view.state.doc.textBetween(from - 1, from, "\n", "\n") : "";
  const lead = before && !/\s/.test(before) ? " " : "";
  const tr = view.state.tr.insertText(lead + text, from, to);
  closeHistory(tr);
  view.dispatch(tr);
  return { from: from + lead.length, to: from + lead.length + text.length };
}

/**
 * Replaces `[from, to)` with `text` as one undo step. Returns the start
 * position of the new text, or `null` if the range is no longer valid.
 */
export function replaceRange(
  view: DictationMarksView,
  from: number,
  to: number,
  text: string,
): number | null {
  if (view.isDestroyed || to <= from || to > view.state.doc.content.size) return null;
  const fragment = textToFragment(view.state.schema, text);
  const tr = view.state.tr.replaceWith(from, to, fragment);
  closeHistory(tr);
  view.dispatch(tr);
  return from;
}

/**
 * Turns a suggested name into an `@name` query with the caret after it; the
 * composer's own autocomplete then opens the normal @ picker for that name.
 */
export function suggestionToPickerQuery(
  view: DictationMarksView & { focus?: () => void },
  from: number,
  to: number,
): boolean {
  if (view.isDestroyed || to <= from) return false;
  const name = view.state.doc.textBetween(from, to, "", "");
  const tr = view.state.tr.insertText(`@${name}`, from, to);
  view.dispatch(tr);
  return true;
}
