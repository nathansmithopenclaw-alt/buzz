import type { DraftMentionRef } from "@/features/messages/lib/useDrafts";
import { formatTeamMention } from "@/features/messages/lib/mentionCandidates";
import type { DictationRosterEntry } from "./dictationRoster";
import type { DictationSegment } from "./mentionResolution";

/** A span of rendered text a mark will be attached to. */
export type RenderedSpan = {
  from: number;
  to: number;
  kind: "mention" | "suggestion";
  entryKey: string;
};

export type RenderedDictation = {
  text: string;
  spans: RenderedSpan[];
  /** Bindings for every mentioned identity (drafts restore these). */
  mentionRefs: DraftMentionRef[];
};

/**
 * Registers a mention and returns the exact label to insert (qualified when
 * another member shares the name), or `null` when it cannot be bound. In a
 * live composer this is `useMentions().registerMentionPubkey`.
 */
export type RegisterMention = (
  displayName: string,
  pubkey: string,
  options: { isAgent: boolean },
) => string | null | undefined;

/**
 * Turns resolved segments into composer text. Mentions use the label their
 * registration returns (the mention-editor contract); a team becomes
 * `Team(@a @b)` with each member bound. A mention at the very end is followed
 * by a space so the caret never sits against it and re-opens the @ picker.
 */
export function renderSegments(
  segments: readonly DictationSegment[],
  register: RegisterMention,
): RenderedDictation {
  let text = "";
  const spans: RenderedSpan[] = [];
  const mentionRefs: DraftMentionRef[] = [];
  const bind = (displayName: string, pubkey: string, isAgent: boolean) => {
    const label = register(displayName, pubkey, { isAgent });
    if (!label) return null;
    mentionRefs.push({ displayName: label, pubkey, isAgent });
    return label;
  };

  for (const segment of segments) {
    if (segment.type === "text") {
      text += segment.text;
      continue;
    }
    if (segment.type === "suggestion") {
      spans.push({
        from: text.length,
        to: text.length + segment.text.length,
        kind: "suggestion",
        entryKey: segment.entry.key,
      });
      text += segment.text;
      continue;
    }
    const rendered = renderMention(segment.entry, bind);
    if (!rendered) {
      text += segment.entry.displayName;
      continue;
    }
    spans.push({
      from: text.length,
      to: text.length + rendered.length,
      kind: "mention",
      entryKey: segment.entry.key,
    });
    text += rendered;
  }
  if (segments[segments.length - 1]?.type === "mention") text += " ";
  return { text, spans, mentionRefs };
}

function renderMention(
  entry: DictationRosterEntry,
  bind: (
    displayName: string,
    pubkey: string,
    isAgent: boolean,
  ) => string | null,
): string | null {
  if (entry.kind === "identity" && entry.pubkey) {
    const label = bind(entry.displayName, entry.pubkey, entry.isAgent);
    return label ? `@${label}` : null;
  }
  if (entry.kind === "team" && entry.teamMembers?.length) {
    const members = [];
    for (const member of entry.teamMembers) {
      if (member.kind !== "identity" || !member.pubkey) return null;
      const label = bind(member.displayName, member.pubkey, entry.isAgent);
      if (!label) return null;
      members.push({ ...member, displayName: label });
    }
    return formatTeamMention(entry.displayName, members).trimEnd();
  }
  // Personas are never auto-mentioned (a persona mention creates an agent).
  return null;
}

/** Replaces the last occurrence of `raw` in `content` (draft delivery). */
export function replaceLastOccurrence(
  content: string,
  raw: string,
  replacement: string,
): string | null {
  const index = content.lastIndexOf(raw);
  if (index < 0 || !raw) return null;
  return (
    content.slice(0, index) + replacement + content.slice(index + raw.length)
  );
}
