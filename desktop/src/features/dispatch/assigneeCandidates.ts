import type { UserSearchResult } from "@/shared/api/types";

function candidateLabel(candidate: UserSearchResult): string | null {
  return candidate.displayName?.trim() || candidate.nip05Handle?.trim() || null;
}

/**
 * Remove directory residue before ranking assignee candidates.
 *
 * A kind:0 profile is append-only identity history, not proof that the key is
 * still a colleague. Community members and Desktop-known agents are the live
 * authority. When several profile keys reuse one visible name, keep every live
 * identity (two current people may genuinely share a name) but collapse a
 * stale-only group to one row. Profiles with no human-readable identity are
 * omitted; direct pubkey paste remains the explicit escape hatch.
 */
export function prepareAssigneeDirectoryCandidates({
  candidates,
  preferredPubkeys,
}: {
  candidates: readonly UserSearchResult[];
  preferredPubkeys: ReadonlySet<string>;
}): UserSearchResult[] {
  const byPubkey = new Map<string, UserSearchResult>();
  for (const candidate of candidates) {
    const pubkey = candidate.pubkey.trim().toLowerCase();
    if (!pubkey || !candidateLabel(candidate) || byPubkey.has(pubkey)) continue;
    byPubkey.set(pubkey, { ...candidate, pubkey });
  }

  const byLabel = new Map<string, UserSearchResult[]>();
  for (const candidate of byPubkey.values()) {
    const label = candidateLabel(candidate);
    if (!label) continue;
    const key = label.toLowerCase();
    const group = byLabel.get(key);
    if (group) group.push(candidate);
    else byLabel.set(key, [candidate]);
  }

  const prepared: UserSearchResult[] = [];
  for (const group of byLabel.values()) {
    const live = group.filter((candidate) =>
      preferredPubkeys.has(candidate.pubkey),
    );
    if (live.length > 0) prepared.push(...live);
    else prepared.push(group[0]);
  }
  return prepared;
}
