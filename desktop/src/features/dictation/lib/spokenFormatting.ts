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
