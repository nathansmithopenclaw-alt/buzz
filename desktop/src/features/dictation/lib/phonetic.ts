/**
 * Sound-alike and near-spelling matching for dictated names.
 *
 * Speech-to-text often misspells roster names ("Quen" for Qwen, "Theeron" for
 * Theron). These are the two measures the dictation benchmark's matcher used
 * (Jaro-Winkler similarity and original Metaphone), ported so the in-app
 * matcher behaves like the one that was measured.
 */

/** Jaro-Winkler similarity in [0, 1]; 1 means identical. Case-sensitive. */
export function jaroWinkler(a: string, b: string): number {
  if (a === b) return 1;
  if (a.length === 0 || b.length === 0) return 0;
  const window = Math.max(0, Math.floor(Math.max(a.length, b.length) / 2) - 1);
  const aMatched = new Array<boolean>(a.length).fill(false);
  const bMatched = new Array<boolean>(b.length).fill(false);
  let matches = 0;
  for (let i = 0; i < a.length; i++) {
    const start = Math.max(0, i - window);
    const end = Math.min(i + window + 1, b.length);
    for (let j = start; j < end; j++) {
      if (bMatched[j] || a[i] !== b[j]) continue;
      aMatched[i] = true;
      bMatched[j] = true;
      matches++;
      break;
    }
  }
  if (matches === 0) return 0;
  let transpositions = 0;
  let k = 0;
  for (let i = 0; i < a.length; i++) {
    if (!aMatched[i]) continue;
    while (!bMatched[k]) k++;
    if (a[i] !== b[k]) transpositions++;
    k++;
  }
  const jaro =
    (matches / a.length +
      matches / b.length +
      (matches - transpositions / 2) / matches) /
    3;
  let prefix = 0;
  while (prefix < 4 && prefix < a.length && prefix < b.length) {
    if (a[prefix] !== b[prefix]) break;
    prefix++;
  }
  return jaro + prefix * 0.1 * (1 - jaro);
}

const VOWELS = "aeiou";

/** Single-character membership; "" (past the end) is never a member. */
const isIn = (ch: string, set: string) => ch.length === 1 && set.includes(ch);

/**
 * Metaphone key for one word, matching the reference implementation the
 * dictation benchmark used (jellyfish's Rust metaphone), quirks included, so
 * in-app matching scores names the way the measured matcher did. Positions
 * past the end of the word read as "". Non-letters are ignored; "0" stands
 * for "th", "X" for "sh".
 */
export function metaphone(input: string): string {
  let s = input
    .normalize("NFKD")
    .toLowerCase()
    .replace(/[^a-z]/g, "");
  if (!s) return "";
  if (["kn", "gn", "pn", "wr", "ae"].some((prefix) => s.startsWith(prefix))) {
    s = s.slice(1);
  }
  const result: string[] = [];
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    const next = s[i + 1] ?? "";
    const nextnext = s[i + 2] ?? "";
    const prev = s[i - 1] ?? "";
    if (c === next && c !== "c") {
      i += 1;
      continue;
    }
    if (isIn(c, VOWELS)) {
      if (i === 0) result.push(c);
    } else if (c === "b") {
      if (!(i !== 0 && prev === "m" && next === "")) result.push("b");
    } else if (c === "c") {
      if ((next === "i" && nextnext === "a") || next === "h") {
        result.push("x");
        i += 1;
      } else if (isIn(next, "iey")) {
        result.push("s");
        i += 1;
      } else {
        result.push("k");
      }
    } else if (c === "d") {
      if (next === "g" && isIn(nextnext, "iey")) {
        result.push("j");
        i += 2;
      } else {
        result.push("t");
      }
    } else if (isIn(c, "fjlmnr")) {
      result.push(c);
    } else if (c === "g") {
      if (isIn(next, "iey")) {
        result.push("j");
      } else if (next === "h" && nextnext !== "" && !isIn(nextnext, VOWELS)) {
        i += 1;
      } else if (next === "n" && nextnext === "") {
        i += 1;
      } else {
        result.push("k");
      }
    } else if (c === "h") {
      if (i === 0 || isIn(next, VOWELS) || !isIn(prev, VOWELS)) {
        result.push("h");
      }
    } else if (c === "k") {
      if (i === 0 || prev !== "c") result.push("k");
    } else if (c === "p") {
      if (next === "h") {
        result.push("f");
        i += 1;
      } else {
        result.push("p");
      }
    } else if (c === "q") {
      result.push("k");
    } else if (c === "s") {
      if (next === "h") {
        result.push("x");
        i += 1;
      } else if (next === "i" && isIn(nextnext, "oa")) {
        result.push("x");
        i += 2;
      } else {
        result.push("s");
      }
    } else if (c === "t") {
      if (next === "i" && isIn(nextnext, "oa")) {
        result.push("x");
      } else if (next === "h") {
        result.push("0");
        i += 1;
      } else if (next !== "c" || nextnext !== "h") {
        result.push("t");
      }
    } else if (c === "v") {
      result.push("f");
    } else if (c === "w") {
      if (i === 0 && next === "h") {
        i += 1;
        result.push("w");
      } else if (isIn(next, VOWELS)) {
        result.push("w");
      }
    } else if (c === "x") {
      if (i === 0) {
        if (next === "h" || (next === "i" && isIn(nextnext, "oa"))) {
          result.push("x");
        } else {
          result.push("s");
        }
      } else {
        result.push("k", "s");
      }
    } else if (c === "y") {
      if (isIn(next, VOWELS)) result.push("y");
    } else if (c === "z") {
      result.push("s");
    }
    i += 1;
  }
  return result.join("").toUpperCase();
}

/** Similarity above which a spoken word is taken to be a roster name. */
export const NAME_MATCH_THRESHOLD = 0.88;
/** Shortest spoken word considered for a fuzzy (non-exact) name match. */
export const MIN_FUZZY_WORD_LENGTH = 4;

/**
 * Similarity between a spoken word and a roster name: the higher of
 * Jaro-Winkler on lowercase text and a fixed 0.9 when the Metaphone keys
 * agree. Exact case-insensitive equality scores 1.
 */
export function nameSimilarity(spoken: string, name: string): number {
  const a = spoken.toLowerCase();
  const b = name.toLowerCase();
  if (a === b) return 1;
  const phonetic = metaphone(a) !== "" && metaphone(a) === metaphone(b);
  return Math.max(jaroWinkler(a, b), phonetic ? 0.9 : 0);
}
