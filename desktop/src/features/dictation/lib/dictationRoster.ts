import type {
  MentionCandidate,
  TeamMentionMember,
} from "@/features/messages/lib/mentionCandidates";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * One nameable party for dictation, derived from the composer's own mention
 * candidates so dictation never sees anyone the @ picker would not offer.
 */
export type DictationRosterEntry = {
  key: string;
  displayName: string;
  /** Lowercase alternative spoken forms (a unique first name for people). */
  aliases: readonly string[];
  kind: "identity" | "persona" | "team";
  pubkey?: string;
  personaId?: string;
  teamId?: string;
  teamMembers?: readonly TeamMentionMember[];
  isAgent: boolean;
  /**
   * May become a real mention without confirmation: a channel member, or a
   * team whose every member is already in the channel. Personas (mentioning
   * one creates an agent), non-members and out-of-channel teams are only ever
   * click-to-confirm suggestions.
   */
  autoMention: boolean;
};

/** Most names sent to the cleanup model. */
export const MAX_MODEL_ROSTER = 150;

export type BuildDictationRosterInput = {
  candidates: readonly MentionCandidate[];
  currentPubkey: string | null;
  memberPubkeys: ReadonlySet<string>;
  /** Recently mentioned pubkeys, most recent first. */
  recentPubkeys?: readonly string[];
};

function teamIsInChannel(
  members: readonly TeamMentionMember[] | undefined,
  memberPubkeys: ReadonlySet<string>,
): boolean {
  if (!members || members.length === 0) return false;
  return members.every(
    (member) =>
      member.kind === "identity" &&
      member.pubkey !== undefined &&
      memberPubkeys.has(normalizePubkey(member.pubkey)),
  );
}

/** Builds dictation roster entries from the composer's mention candidates. */
export function buildDictationRoster({
  candidates,
  currentPubkey,
  memberPubkeys,
  recentPubkeys = [],
}: BuildDictationRosterInput): DictationRosterEntry[] {
  const self = currentPubkey ? normalizePubkey(currentPubkey) : null;
  const normalizedMembers = new Set(
    [...memberPubkeys].map((pubkey) => normalizePubkey(pubkey)),
  );
  const entries: DictationRosterEntry[] = [];
  const seen = new Set<string>();
  for (const candidate of candidates) {
    const displayName = candidate.displayName?.trim();
    if (!displayName || candidate.isGlobalSearchResult) continue;
    if (candidate.kind === "identity") {
      if (!candidate.pubkey) continue;
      const pubkey = normalizePubkey(candidate.pubkey);
      if (pubkey === self || seen.has(`p:${pubkey}`)) continue;
      seen.add(`p:${pubkey}`);
      entries.push({
        key: `p:${pubkey}`,
        displayName,
        aliases: [],
        kind: "identity",
        pubkey,
        isAgent: candidate.isAgent,
        autoMention: candidate.isMember,
      });
    } else if (candidate.kind === "persona") {
      if (!candidate.personaId || seen.has(`persona:${candidate.personaId}`)) {
        continue;
      }
      seen.add(`persona:${candidate.personaId}`);
      entries.push({
        key: `persona:${candidate.personaId}`,
        displayName,
        aliases: [],
        kind: "persona",
        personaId: candidate.personaId,
        isAgent: true,
        autoMention: false,
      });
    } else {
      if (!candidate.teamId || seen.has(`team:${candidate.teamId}`)) continue;
      seen.add(`team:${candidate.teamId}`);
      entries.push({
        key: `team:${candidate.teamId}`,
        displayName,
        aliases: [],
        kind: "team",
        teamId: candidate.teamId,
        teamMembers: candidate.teamMembers,
        isAgent: candidate.isAgent,
        autoMention: teamIsInChannel(candidate.teamMembers, normalizedMembers),
      });
    }
  }
  return orderByRecency(withFirstNameAliases(entries), recentPubkeys);
}

/** People are addressed by first name; add it when no one else shares it. */
function withFirstNameAliases(
  entries: DictationRosterEntry[],
): DictationRosterEntry[] {
  const taken = new Map<string, number>();
  const bump = (name: string) => taken.set(name, (taken.get(name) ?? 0) + 1);
  for (const entry of entries) {
    bump(entry.displayName.toLowerCase());
    const first = entry.displayName.split(/\s+/)[0]?.toLowerCase();
    if (first && first !== entry.displayName.toLowerCase()) bump(first);
  }
  return entries.map((entry) => {
    if (entry.kind !== "identity" || entry.isAgent) return entry;
    const first = entry.displayName.split(/\s+/)[0]?.toLowerCase();
    if (!first || first === entry.displayName.toLowerCase()) return entry;
    return taken.get(first) === 1 ? { ...entry, aliases: [first] } : entry;
  });
}

function orderByRecency(
  entries: DictationRosterEntry[],
  recentPubkeys: readonly string[],
): DictationRosterEntry[] {
  const rank = new Map(
    recentPubkeys.map((pubkey, index) => [normalizePubkey(pubkey), index]),
  );
  return entries
    .map((entry, index) => ({ entry, index }))
    .sort((a, b) => {
      const ra = a.entry.pubkey ? (rank.get(a.entry.pubkey) ?? Infinity) : Infinity;
      const rb = b.entry.pubkey ? (rank.get(b.entry.pubkey) ?? Infinity) : Infinity;
      if (ra !== rb) return ra - rb;
      if (a.entry.autoMention !== b.entry.autoMention) {
        return a.entry.autoMention ? -1 : 1;
      }
      return a.index - b.index;
    })
    .map(({ entry }) => entry);
}

/**
 * Names the cleanup model may mention: auto-mentionable entries only, most
 * relevant first, capped. Everyone else is resolved by Buzz as suggestions.
 */
export function modelRosterNames(
  entries: readonly DictationRosterEntry[],
): string[] {
  const names: string[] = [];
  const seen = new Set<string>();
  for (const entry of entries) {
    if (!entry.autoMention) continue;
    const key = entry.displayName.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    names.push(entry.displayName);
    if (names.length === MAX_MODEL_ROSTER) break;
  }
  return names;
}
