import { closeHistory } from "@tiptap/pm/history";
import { Fragment, type Node, type Schema } from "@tiptap/pm/model";
import { TextSelection } from "@tiptap/pm/state";

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
 * one undo step, adding a separating space after a word. "\n" becomes a hard
 * break (one position, like the character), so offsets into `text` stay
 * valid document offsets from the returned `from`. Returns the range the
 * words occupy.
 */
export function insertAtSelection(
  view: DictationMarksView,
  text: string,
): { from: number; to: number } | null {
  if (view.isDestroyed || !text) return null;
  const { from, to } = view.state.selection;
  const before =
    from > 1 ? view.state.doc.textBetween(from - 1, from, "\n", "\n") : "";
  const lead = before && !/\s/.test(before) ? " " : "";
  const tr = view.state.tr.replaceWith(
    from,
    to,
    textToFragment(view.state.schema, lead + text),
  );
  const end = from + lead.length + text.length;
  tr.setSelection(TextSelection.create(tr.doc, end));
  closeHistory(tr);
  view.dispatch(tr);
  return { from: from + lead.length, to: end };
}

/**
 * Every place `text` (with "\n" as hard breaks, as `textToFragment` writes
 * it) appears as contiguous inline content, as document ranges.
 */
export function findTextRanges(
  doc: Node,
  text: string,
): { from: number; to: number }[] {
  const ranges: { from: number; to: number }[] = [];
  if (!text) return ranges;
  doc.descendants((node, pos) => {
    if (!node.isTextblock) return true;
    let flat = "";
    const positions: number[] = [];
    node.forEach((child, offset) => {
      const start = pos + 1 + offset;
      if (child.isText) {
        const value = child.text ?? "";
        for (let i = 0; i < value.length; i++) positions.push(start + i);
        flat += value;
      } else {
        positions.push(start);
        flat += child.type.name === "hardBreak" ? "\n" : "\uFFFC";
      }
    });
    for (let i = flat.indexOf(text); i >= 0; i = flat.indexOf(text, i + 1)) {
      ranges.push({
        from: positions[i],
        to: positions[i + text.length - 1] + 1,
      });
    }
    return false;
  });
  return ranges;
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
  if (view.isDestroyed || to <= from || to > view.state.doc.content.size)
    return null;
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
  tr.setSelection(TextSelection.create(tr.doc, from + name.length + 1));
  view.dispatch(tr);
  return true;
}
