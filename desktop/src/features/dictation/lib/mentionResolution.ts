import type { DictationRosterEntry } from "./dictationRoster";
import {
  MIN_FUZZY_WORD_LENGTH,
  NAME_MATCH_THRESHOLD,
  nameSimilarity,
} from "./phonetic";

/**
 * Dictated text resolved into plain text, real mentions, and click-to-confirm
 * suggestions. Buzz, not the model, decides what becomes a mention: model
 * markers are validated against the roster, and everything else is at most a
 * suggestion the user confirms through the normal @ picker.
 */
export type DictationSegment =
  | { type: "text"; text: string }
  | { type: "mention"; entry: DictationRosterEntry }
  | { type: "suggestion"; entry: DictationRosterEntry; text: string };

const MARKER = /@\[([^\]\n]{1,80})\]/g;
const MAX_NAME_WORDS = 4;
const CUE_WORDS = new Set([
  "hey",
  "hi",
  "hello",
  "ok",
  "okay",
  "ask",
  "tell",
  "ping",
  "cc",
  "remind",
  "thanks",
  "morning",
]);
/** Words that open a greeting slot: the next name is spoken to. */
const GREETINGS = new Set([
  "hey",
  "hi",
  "hello",
  "ok",
  "okay",
  "morning",
  "thanks",
]);
const STOPWORDS = new Set([
  "the",
  "then",
  "there",
  "they",
  "them",
  "this",
  "that",
  "and",
  "can",
  "you",
  "your",
  "for",
  "from",
  "with",
  "what",
  "when",
  "will",
  "well",
  "hey",
  "all",
  "our",
  "one",
  "have",
  "here",
  "just",
  "into",
]);

type Lookup =
  | { type: "none" }
  | { type: "one"; entry: DictationRosterEntry; score?: number }
  | { type: "ambiguous" };

/** Fuzzy matches this close count in the strongest addressing slots. */
const SLOT_MATCH_THRESHOLD = 0.84;
/** Shortest heard word a strong slot may fuzzy-match ("Ren" → Wren). */
const SLOT_MIN_FUZZY_LENGTH = 3;

/** Exact (case-insensitive) display name first, then a unique alias. */
function lookupExact(
  name: string,
  roster: readonly DictationRosterEntry[],
): Lookup {
  const key = name.trim().toLowerCase();
  if (!key) return { type: "none" };
  const byName = roster.filter((e) => e.displayName.toLowerCase() === key);
  if (byName.length === 1) return { type: "one", entry: byName[0] };
  if (byName.length > 1) return { type: "ambiguous" };
  const byAlias = roster.filter((e) => e.aliases.includes(key));
  if (byAlias.length === 1) return { type: "one", entry: byAlias[0] };
  return byAlias.length > 1 ? { type: "ambiguous" } : { type: "none" };
}

/** Best fuzzy single-word match, or none when no entry clears the threshold. */
function lookupFuzzy(
  word: string,
  roster: readonly DictationRosterEntry[],
  threshold = NAME_MATCH_THRESHOLD,
  minLength = MIN_FUZZY_WORD_LENGTH,
): Lookup {
  if (word.length < minLength || STOPWORDS.has(word.toLowerCase())) {
    return { type: "none" };
  }
  let best: DictationRosterEntry | null = null;
  let bestScore = 0;
  let tie = false;
  for (const entry of roster) {
    for (const name of [entry.displayName, ...entry.aliases]) {
      if (/\s/.test(name)) continue;
      const score = nameSimilarity(word, name);
      if (score > bestScore) {
        best = entry;
        bestScore = score;
        tie = false;
      } else if (score === bestScore && best && best.key !== entry.key) {
        tie = true;
      }
    }
  }
  if (!best || bestScore < threshold) return { type: "none" };
  return tie
    ? { type: "ambiguous" }
    : { type: "one", entry: best, score: bestScore };
}

type Token = { raw: string; core: string; start: number; end: number };

const LEADING = /^["'([]+/;
const TRAILING = /[,.!?;:"')\]]+$/;

function tokenize(text: string): Token[] {
  const tokens: Token[] = [];
  for (const match of text.matchAll(/\S+/g)) {
    const raw = match[0];
    const start = match.index ?? 0;
    const lead = raw.match(LEADING)?.[0].length ?? 0;
    const trail = raw.match(TRAILING)?.[0].length ?? 0;
    const core = raw.slice(lead, raw.length - trail);
    tokens.push({
      raw,
      core,
      start: start + lead,
      end: start + raw.length - trail,
    });
  }
  return tokens;
}

const endsClause = (token: Token | undefined) =>
  token === undefined || /[.!?;:,]["')\]]*$/.test(token.raw);
const endsSentence = (token: Token | undefined) =>
  token !== undefined && /[.!?]["')\]]*$/.test(token.raw);
const lower = (token: Token | undefined) => token?.core.toLowerCase() ?? "";
const isPossessive = (core: string) => /['’]s$/i.test(core);
const trailingPunct = (token: Token) => token.raw.match(TRAILING)?.[0] ?? "";

export type AddressHit = {
  start: number;
  end: number;
  /** Where the hit's text begins including a spoken "at" (for "at" hits). */
  consumeFrom: number;
  entry: DictationRosterEntry;
  /** "at" = explicit "at <name>" request; "address" = spoken to them. */
  how: "at" | "address";
};

/**
 * Finds names spoken *to* someone in plain text. Talking *about* someone
 * (possessives, mid-sentence references) is never a hit, and a lowercase
 * ordinary word ("patch the bug") only counts after an explicit "at".
 */
export function findAddressedNames(
  text: string,
  roster: readonly DictationRosterEntry[],
): AddressHit[] {
  const tokens = tokenize(text);
  const hits: AddressHit[] = [];
  let lastHitTokenEnd = -1;
  let lastHitWasGreeting = false;
  for (let i = 0; i < tokens.length; i++) {
    let span = 0;
    let lookup: Lookup = { type: "none" };
    for (let n = Math.min(MAX_NAME_WORDS, tokens.length - i); n >= 1; n--) {
      const words = tokens.slice(i, i + n);
      if (words.slice(0, -1).some((t) => TRAILING.test(t.raw))) continue;
      const phrase = words.map((t) => t.core).join(" ");
      lookup = lookupExact(isPossessive(phrase) ? "" : phrase, roster);
      if (lookup.type !== "none") {
        span = n;
        break;
      }
    }
    let fuzzy = false;
    if (lookup.type === "none") {
      lookup = lookupFuzzy(
        tokens[i].core,
        roster,
        SLOT_MATCH_THRESHOLD,
        SLOT_MIN_FUZZY_LENGTH,
      );
      span = 1;
      fuzzy = true;
    }
    if (lookup.type !== "one") continue;
    const first = tokens[i];
    const last = tokens[i + span - 1];
    if (isPossessive(last.core)) continue;
    const prev = tokens[i - 1];
    const next = tokens[i + span];
    const punct = trailingPunct(last);
    const capitalized = /^\p{Lu}/u.test(first.core);
    const strictFuzzy = !fuzzy || (lookup.score ?? 0) >= NAME_MATCH_THRESHOLD;
    const longEnough = first.core.length >= MIN_FUZZY_WORD_LENGTH;

    const atRule =
      lower(prev) === "at" &&
      !TRAILING.test(prev?.raw ?? "") &&
      (endsClause(tokens[i - 2]) || lower(tokens[i - 2]) === "and") &&
      (punct.length > 0 || capitalized);

    // Addressing slots, strongest first.
    const sentenceStart = endsSentence(prev) || prev === undefined;
    const listContinues = lower(next) === "and" || lower(next) === "&";
    const opening =
      endsClause(prev) &&
      (punct.startsWith(",") ||
        punct.startsWith(":") ||
        listContinues ||
        (sentenceStart && /^[.!]/.test(punct)));
    const greeting =
      GREETINGS.has(lower(prev)) ||
      (lower(tokens[i - 2]) === "thank" && lower(prev) === "you") ||
      (GREETINGS.has(lower(tokens[i - 2])) && lower(prev) === "there");
    const afterLastHit = (k: number) => k >= 0 && k === lastHitTokenEnd;
    const joined =
      (afterLastHit(i - 1) && /,$/.test(prev?.raw ?? "")) ||
      ((lower(prev) === "and" || lower(prev) === "&") && afterLastHit(i - 2));
    // A list of addressees ends in a pause ("…and Nectar, standup…") or
    // continues; "Nemo, Morty wants…" is Nemo being told about Morty. After a
    // greeting the list may run on ("Thanks Theron, Vela and Sable for…").
    const list =
      joined && (/^[,:.!?]/.test(punct) || listContinues || lastHitWasGreeting);
    const trailing = /[.!?]/.test(punct) && /,$/.test(prev?.raw ?? "");
    const relay =
      (CUE_WORDS.has(lower(prev)) && !endsSentence(prev)) ||
      (lower(tokens[i - 2]) === "loop" && lower(prev) === "in");

    // Capitalized exact names may use any slot. A lowercase exact name
    // ("patch, rebase…") needs a strong slot. A near-miss spelling ("Quen")
    // needs a strong slot, and the loosest spellings only the safest ones.
    const strongSlot = opening || greeting || list || trailing;
    const safestSlot = greeting || list || trailing;
    let how: AddressHit["how"] | null = atRule ? "at" : null;
    if (!how) {
      const accepted = fuzzy
        ? capitalized &&
          ((strictFuzzy && longEnough && (strongSlot || relay)) || safestSlot)
        : capitalized
          ? strongSlot || relay
          : strongSlot && sentenceStart
            ? true
            : safestSlot || relay;
      if (accepted) how = "address";
    }
    if (!how) continue;
    hits.push({
      start: first.start,
      end: last.end,
      consumeFrom: how === "at" && prev ? prev.start : first.start,
      entry: lookup.entry,
      how,
    });
    lastHitTokenEnd = i + span - 1;
    lastHitWasGreeting = greeting || (list && lastHitWasGreeting);
    i += span - 1;
  }
  return hits;
}

function pushText(segments: DictationSegment[], text: string) {
  if (!text) return;
  const last = segments[segments.length - 1];
  if (last?.type === "text") last.text += text;
  else segments.push({ type: "text", text });
}

/**
 * Splits plain text around addressed names: an explicit "at <name>" to an
 * auto-mentionable entry becomes a mention; every other hit is a suggestion.
 */
function splitAddressed(
  text: string,
  roster: readonly DictationRosterEntry[],
): DictationSegment[] {
  const segments: DictationSegment[] = [];
  let cursor = 0;
  for (const hit of findAddressedNames(text, roster)) {
    const atMention = hit.entry.autoMention;
    // The spoken "at" is consumed by the mention itself.
    const start = atMention ? hit.consumeFrom : hit.start;
    pushText(segments, text.slice(cursor, Math.max(cursor, start)));
    if (atMention) segments.push({ type: "mention", entry: hit.entry });
    else {
      segments.push({
        type: "suggestion",
        entry: hit.entry,
        text: text.slice(hit.start, hit.end),
      });
    }
    cursor = hit.end;
  }
  pushText(segments, text.slice(cursor));
  return segments;
}

/**
 * Resolves cleanup-model output. Each `@[Name]` marker becomes a real mention
 * only for a single auto-mentionable roster entry; personas, non-members and
 * out-of-channel teams become suggestions; unknown or ambiguous names become
 * plain text. Addressed names the model left unmarked become suggestions.
 *
 * Address detection runs over the whole sentence (markers unwrapped to their
 * names) so context such as "@[Anvil], and Outsider, …" carries across.
 */
export function resolveModelOutput(
  text: string,
  roster: readonly DictationRosterEntry[],
): DictationSegment[] {
  type EntrySegment = Exclude<DictationSegment, { type: "text" }>;
  type Marker = { start: number; end: number; segment: EntrySegment | null };
  const markers: Marker[] = [];
  let plain = "";
  let cursor = 0;
  for (const match of text.matchAll(MARKER)) {
    const index = match.index ?? 0;
    plain += text.slice(cursor, index);
    const name = match[1].trim();
    const lookup = lookupExact(name, roster);
    const segment: EntrySegment | null =
      lookup.type !== "one"
        ? null
        : lookup.entry.autoMention
          ? { type: "mention", entry: lookup.entry }
          : { type: "suggestion", entry: lookup.entry, text: name };
    markers.push({
      start: plain.length,
      end: plain.length + name.length,
      segment,
    });
    plain += name;
    cursor = index + match[0].length;
  }
  plain += text.slice(cursor);

  const taken = new Set(
    markers.flatMap((m) => (m.segment ? [m.segment.entry.key] : [])),
  );
  // Only resolved markers own their text; an unresolved one is plain text
  // that may still be recognised as an addressed name.
  const overlapsMarker = (start: number, end: number) =>
    markers.some((m) => m.segment && start < m.end && end > m.start);
  const suggestions = findAddressedNames(plain, roster).filter(
    (hit) => !taken.has(hit.entry.key) && !overlapsMarker(hit.start, hit.end),
  );

  type Span = { start: number; end: number; segment: DictationSegment };
  const spans: Span[] = [
    ...markers.flatMap((m) =>
      m.segment ? [{ start: m.start, end: m.end, segment: m.segment }] : [],
    ),
    ...suggestions.map((hit) => ({
      start: hit.start,
      end: hit.end,
      segment: {
        type: "suggestion" as const,
        entry: hit.entry,
        text: plain.slice(hit.start, hit.end),
      },
    })),
  ].sort((a, b) => a.start - b.start);

  const segments: DictationSegment[] = [];
  let at = 0;
  for (const span of spans) {
    pushText(segments, plain.slice(at, span.start));
    segments.push(span.segment);
    at = span.end;
  }
  pushText(segments, plain.slice(at));
  return segments;
}

/**
 * Resolves text with no cleanup model (Verbatim, or any fallback): only an
 * explicit "at <name>" to an auto-mentionable entry becomes a mention; every
 * other addressed name is a suggestion.
 */
export function resolveVerbatim(
  text: string,
  roster: readonly DictationRosterEntry[],
): DictationSegment[] {
  return splitAddressed(text, roster);
}
