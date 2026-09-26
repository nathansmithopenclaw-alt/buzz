import type { DictationSegment } from "./mentionResolution";

/**
 * Polish: an optional model pass over a dictation that is already in the
 * composer with its mentions. Every mention and suggestion reaches the model
 * as a `{{n}}` placeholder, and a reply is used only if it keeps the
 * speaker's words in order — dropping fillers, false starts and retractions
 * and fixing punctuation, case and number formatting — with every
 * placeholder intact. Anything else keeps the dictation as inserted.
 */

type EntrySegment = Exclude<DictationSegment, { type: "text" }>;

export type PolishRequest = {
  /** The dictation with each mention or suggestion as `{{n}}`. */
  text: string;
  entries: EntrySegment[];
};

const PLACEHOLDER = /\{\{(\d+)\}\}/g;
/** A spoken correction right after a name retracts it ("tell X, no, Y"). */
const RETRACTION_CUE =
  /^[,.]?\s*(?:no|sorry|i mean|wait|or rather|actually|scratch that|no wait)\b/i;
/** Below this share of the speaker's words, a reply is a rewrite, not a polish. */
const MIN_KEPT_WORDS = 0.4;

const entryText = (segment: EntrySegment) =>
  segment.type === "mention" ? segment.entry.displayName : segment.text;

export function toPolishRequest(
  segments: readonly DictationSegment[],
): PolishRequest {
  let text = "";
  const entries: EntrySegment[] = [];
  for (const segment of segments) {
    if (segment.type === "text") {
      text += segment.text;
      continue;
    }
    entries.push(segment);
    text += `{{${entries.length}}}`;
  }
  return { text, entries };
}

function stripWrappers(reply: string): string {
  let out = reply.trim();
  if (out.startsWith("```") && out.endsWith("```")) {
    out = out.replace(/^`+|`+$/g, "").trim();
  }
  if (out.length > 1 && out.startsWith('"') && out.endsWith('"')) {
    const inner = out.slice(1, -1);
    if (!inner.includes('"')) out = inner;
  }
  return out.trim();
}

function counts(text: string): Map<string, number> {
  const found = new Map<string, number>();
  for (const match of text.matchAll(PLACEHOLDER)) {
    found.set(match[1], (found.get(match[1]) ?? 0) + 1);
  }
  return found;
}

/**
 * Every placeholder kept exactly once, except one the speaker retracted may
 * be dropped. None added, none duplicated, no stray braces.
 */
function placeholdersKept(input: string, reply: string): boolean {
  const before = counts(input);
  const after = counts(reply);
  const bare = reply.replace(PLACEHOLDER, "");
  if (bare.includes("{{") || bare.includes("}}")) return false;
  for (const [key, count] of after) {
    if (count > (before.get(key) ?? 0)) return false;
  }
  for (const match of input.matchAll(PLACEHOLDER)) {
    if ((after.get(match[1]) ?? 0) >= (before.get(match[1]) ?? 0)) continue;
    const rest = input.slice((match.index ?? 0) + match[0].length);
    if (!RETRACTION_CUE.test(rest)) return false;
  }
  return true;
}

// ── Word comparison (numbers normalized: "7:30" matches "seven thirty") ──────

const ONES =
  "zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen".split(
    " ",
  );
const TENS = "_ _ twenty thirty forty fifty sixty seventy eighty ninety".split(
  " ",
);
const EQUIVALENT: Record<string, string> = {
  ok: "okay",
  standup: "stand up",
  alright: "all right",
};

function numberWords(n: number): string {
  if (n < 20) return ONES[n];
  if (n < 100)
    return TENS[Math.floor(n / 10)] + (n % 10 ? ` ${ONES[n % 10]}` : "");
  if (n < 1000) {
    const rest = n % 100;
    return `${ONES[Math.floor(n / 100)]} hundred${rest ? ` ${numberWords(rest)}` : ""}`;
  }
  if (n < 1_000_000) {
    const rest = n % 1000;
    return `${numberWords(Math.floor(n / 1000))} thousand${rest ? ` ${numberWords(rest)}` : ""}`;
  }
  return String(n);
}

export function contentWords(text: string): string[] {
  let s = text
    .replace(/[‘’]/g, "'")
    .replace(/[–—]/g, "-")
    .toLowerCase()
    .replace(/(?<=\d),(?=\d{3}\b)/g, "")
    .replaceAll("%", " percent ")
    .replace(/\$(\d+)/g, " $1 dollars");
  for (let i = 0; i < 3; i++) s = s.replace(/(\d)\.(\d)/g, "$1 point $2");
  s = s.replace(/(\d):(\d\d)/g, "$1 $2").replace(/[-/]/g, " ");
  const words: string[] = [];
  for (const raw of s.match(/[a-z0-9']+/g) ?? []) {
    const token = raw.replace(/^'+|'+$/g, "");
    if (!token) continue;
    if (/^\d+$/.test(token))
      words.push(...numberWords(Number(token)).split(" "));
    else words.push(...(EQUIVALENT[token] ?? token).split(" "));
  }
  return words;
}

/**
 * The reply's words are the source's words in order with only deletions; a
 * joined compound ("stand up" → "standup") counts as its two source words.
 */
export function isEditSubsequence(reply: string, source: string): boolean {
  const out = contentWords(reply);
  const src = contentWords(source);
  let j = 0;
  for (const word of out) {
    while (
      j < src.length &&
      src[j] !== word &&
      !(j + 1 < src.length && src[j] + src[j + 1] === word)
    ) {
      j += 1;
    }
    if (j >= src.length) return false;
    j += src[j] === word ? 1 : 2;
  }
  return true;
}

function unwrap(text: string, entries: readonly EntrySegment[]): string {
  return text.replace(PLACEHOLDER, (whole, n: string) => {
    const entry = entries[Number(n) - 1];
    return entry ? entryText(entry) : whole;
  });
}

export type PolishVerdict =
  | { accepted: true; segments: DictationSegment[] }
  | { accepted: false; reason: string };

/** Checks a model reply against the request and rebuilds the segments. */
export function acceptPolish(
  request: PolishRequest,
  reply: string,
): PolishVerdict {
  const text = stripWrappers(reply);
  if (!text) return { accepted: false, reason: "empty" };
  if (!placeholdersKept(request.text, text)) {
    return { accepted: false, reason: "placeholders" };
  }
  const source = unwrap(request.text, request.entries);
  const polished = unwrap(text, request.entries);
  if (!isEditSubsequence(polished, source)) {
    return { accepted: false, reason: "reworded" };
  }
  const sourceWords = contentWords(source).length;
  if (
    sourceWords >= 5 &&
    contentWords(polished).length < MIN_KEPT_WORDS * sourceWords
  ) {
    return { accepted: false, reason: "over-deleted" };
  }
  const segments: DictationSegment[] = [];
  let at = 0;
  for (const match of text.matchAll(PLACEHOLDER)) {
    const index = match.index ?? 0;
    if (index > at)
      segments.push({ type: "text", text: text.slice(at, index) });
    const entry = request.entries[Number(match[1]) - 1];
    if (entry) segments.push(entry);
    at = index + match[0].length;
  }
  if (at < text.length) segments.push({ type: "text", text: text.slice(at) });
  return { accepted: true, segments };
}
