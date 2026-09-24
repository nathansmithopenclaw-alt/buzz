import { Extension } from "@tiptap/core";
import {
  Plugin,
  PluginKey,
  type EditorState,
  type Transaction,
} from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

/**
 * Composer ranges that belong to a dictation:
 *
 * - `pending`: raw words shown greyed while cleanup runs. The swap to cleaned
 *   text only happens while this range is intact, so a late cleanup result can
 *   never overwrite text the user has since edited.
 * - `suggestion`: a name Buzz thinks was addressed but will not mention on its
 *   own; clicking it opens the normal @ picker for that name.
 * - `flash`: a brief highlight on mentions dictation just inserted, so a wrong
 *   one is noticed before sending (a mention can wake an agent).
 *
 * Ranges follow the same rule as pasted-mention occurrences: edits outside a
 * range move it, any edit overlapping it kills it (fail closed).
 */
export const dictationMarksKey = new PluginKey<DictationMarks>(
  "dictationMarks",
);

export type DictationMarkKind = "pending" | "suggestion" | "flash";

export type DictationMark = {
  from: number;
  to: number;
  kind: DictationMarkKind;
  /** Roster entry key for suggestions. */
  entryKey?: string;
};

type DictationMarks = ReadonlyMap<number, DictationMark>;

type DictationMarkCommand =
  | { type: "track"; id: number; mark: DictationMark }
  | { type: "release"; id: number };

/** The slice of `EditorView` these helpers need (tests pass a bare state). */
export type DictationMarksView = {
  state: EditorState;
  dispatch: (tr: Transaction) => void;
  isDestroyed?: boolean;
};

/** Backstop so a composer can never accumulate marks for its lifetime. */
export const MAX_DICTATION_MARKS = 60;

/** Name of the DOM event fired when a suggestion is clicked. */
export const DICTATION_SUGGESTION_EVENT = "buzz-dictation-suggestion";

let nextMarkId = 1;

function remap(mark: DictationMark, tr: Transaction): DictationMark | null {
  let { from, to } = mark;
  for (const map of tr.mapping.maps) {
    let overlapped = false;
    map.forEach((oldStart, oldEnd) => {
      if (oldEnd > oldStart && oldStart < to && oldEnd > from)
        overlapped = true;
    });
    if (overlapped) return null;
    const mappedFrom = map.mapResult(from, 1);
    const mappedTo = map.mapResult(to, -1);
    if (mappedFrom.deleted || mappedTo.deleted) return null;
    from = mappedFrom.pos;
    to = mappedTo.pos;
    if (to <= from) return null;
  }
  return { ...mark, from, to };
}

function apply(current: DictationMarks, tr: Transaction): DictationMarks {
  let next: Map<number, DictationMark> | null = null;
  if (tr.docChanged) {
    next = new Map();
    for (const [id, mark] of current) {
      const mapped = remap(mark, tr);
      if (mapped) next.set(id, mapped);
    }
  }
  const command = tr.getMeta(dictationMarksKey) as
    | DictationMarkCommand
    | undefined;
  if (!command) return next ?? current;
  next ??= new Map(current);
  if (command.type === "release") {
    next.delete(command.id);
  } else {
    next.set(command.id, command.mark);
    for (const id of next.keys()) {
      if (next.size <= MAX_DICTATION_MARKS) break;
      next.delete(id);
    }
  }
  return next;
}

const CLASS: Record<DictationMarkKind, string> = {
  pending: "dictation-pending",
  suggestion: "dictation-suggestion",
  flash: "dictation-flash",
};

function decorations(state: EditorState): DecorationSet {
  const marks = dictationMarksKey.getState(state);
  if (!marks || marks.size === 0) return DecorationSet.empty;
  const size = state.doc.content.size;
  const items: Decoration[] = [];
  for (const [id, mark] of marks) {
    if (mark.to > size) continue;
    const attrs: Record<string, string> = { class: CLASS[mark.kind] };
    if (mark.kind === "pending") attrs["aria-busy"] = "true";
    if (mark.kind === "suggestion") {
      attrs["data-dictation-suggestion"] = String(id);
      attrs.title = "Suggested mention — click to choose who you meant";
    }
    items.push(Decoration.inline(mark.from, mark.to, attrs));
  }
  return DecorationSet.create(state.doc, items);
}

/** Tracks dictation ranges and renders them; registered on every composer. */
export const DictationMarksExtension = Extension.create({
  name: "dictationMarks",
  addProseMirrorPlugins() {
    return [
      new Plugin<DictationMarks>({
        key: dictationMarksKey,
        state: {
          init: () => new Map(),
          apply: (tr, current) => apply(current, tr),
        },
        props: {
          decorations,
          handleClickOn(view, _pos, _node, _nodePos, event) {
            const target = event.target as HTMLElement | null;
            const hit = target?.closest?.("[data-dictation-suggestion]");
            const id = Number(hit?.getAttribute("data-dictation-suggestion"));
            if (!hit || !Number.isFinite(id)) return false;
            view.dom.dispatchEvent(
              new CustomEvent(DICTATION_SUGGESTION_EVENT, {
                bubbles: true,
                detail: { id },
              }),
            );
            return true;
          },
        },
      }),
    ];
  },
});

/** Starts tracking `[from, to)`; `null` when empty or the plugin is absent. */
export function trackDictationMark(
  view: DictationMarksView,
  mark: DictationMark,
): number | null {
  if (view.isDestroyed || mark.to <= mark.from) return null;
  if (!dictationMarksKey.getState(view.state)) return null;
  const id = nextMarkId++;
  view.dispatch(
    view.state.tr
      .setMeta(dictationMarksKey, { type: "track", id, mark })
      .setMeta("addToHistory", false),
  );
  return id;
}

/** Current range of a mark, or `null` once it is gone. */
export function readDictationMark(
  view: DictationMarksView,
  id: number | null,
): DictationMark | null {
  if (id === null || view.isDestroyed) return null;
  const mark = dictationMarksKey.getState(view.state)?.get(id);
  if (!mark) return null;
  return mark.to > view.state.doc.content.size ? null : mark;
}

/** Stops tracking a mark (no-op when already gone). */
export function releaseDictationMark(
  view: DictationMarksView,
  id: number | null,
): void {
  if (id === null || view.isDestroyed) return;
  if (!dictationMarksKey.getState(view.state)?.has(id)) return;
  view.dispatch(
    view.state.tr
      .setMeta(dictationMarksKey, { type: "release", id })
      .setMeta("addToHistory", false),
  );
}
