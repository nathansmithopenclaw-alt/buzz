/**
 * Spoken formatting commands in dictated text.
 *
 * "new line" / "new paragraph" become line breaks only when spoken as a
 * command: on their own between sentence or clause boundaries ("…deploy. New
 * paragraph. Next…"). Ordinary phrases such as "a new line of products" are
 * left alone.
 */

const COMMAND =
  /(^|[.!?,;:])\s*\b(new line|new paragraph)\b[.!?,;:]?(?=\s|$)\s*/gi;

/** Replaces spoken formatting commands with line breaks. */
export function applySpokenFormatting(text: string): string {
  const replaced = text.replace(
    COMMAND,
    (_match, boundary: string, command: string) => {
      const breakText =
        command.toLowerCase() === "new paragraph" ? "\n\n" : "\n";
      return `${boundary}${breakText}`;
    },
  );
  // Capitalize the first letter after an inserted break.
  return replaced
    .replace(/\n([a-z])/g, (_m, letter: string) => `\n${letter.toUpperCase()}`)
    .replace(/^\n+/, "")
    .replace(/\n+$/, "");
}

/**
 * Spoken self-corrections that cancel what came before them in the sentence.
 * "never mind" only counts when it stands alone ("…, never mind, …"), so
 * "never mind the cost" stays as spoken.
 */
const CORRECTION =
  /(^|[.!?]\s+)[^.!?]*?\b(?:(?:scratch that|no wait|wait no)\b[,.;:!?]*|(?:actually )?never mind[,.;:!?]+)\s*/gi;

/** A correction that starts its own sentence, with the sentence before it. */
const OPENING_CORRECTION =
  /(^|[.!?]\s+)[^.!?]*[.!?]\s+(?:scratch that|no wait|wait no|(?:actually )?never mind)\b[,.;:!?]*\s*/gi;

/**
 * Filler words, when followed by punctuation or a lowercase word ("um, so",
 * "uh the"). "uh-oh" and a capitalized name like "Um Kulthum" are kept.
 */
const FILLER =
  /\b(?:[Uu]m+|[Uu]h+|[Ee]rm*|[Aa]h+|[Hh]mm+)\b(?!-)(?:[,.]\s*|\s+(?=\p{Ll}))/gu;

/** "like" used as a filler, bounded by commas on both sides. */
const LIKE_FILLER = /,\s*like,\s*/gi;

/**
 * Local cleanup that never needs a model: drops filler words and applies
 * spoken self-corrections ("Anvil, scratch that, Bolt, can you…" keeps only
 * "Bolt, can you…"), then restores sentence capitalization.
 */
export function applySpokenCorrections(text: string): string {
  const corrected = text
    // "Anvil. Scratch that. Bolt…": speech split into sentences, so a
    // correction that opens a sentence cancels the one before it.
    .replace(OPENING_CORRECTION, (_match, boundary: string) => boundary)
    .replace(CORRECTION, (_match, boundary: string) => boundary)
    .replace(FILLER, "")
    .replace(LIKE_FILLER, " ")
    .replace(/\s+([,.!?])/g, "$1")
    .replace(/,\s*,/g, ",")
    .replace(/^[,\s]+/, "")
    .replace(/\s{2,}/g, " ");
  return corrected
    .replace(
      /(^|[.!?]\s+)([a-z])/g,
      (_m, lead: string, letter: string) => `${lead}${letter.toUpperCase()}`,
    )
    .trim();
}
