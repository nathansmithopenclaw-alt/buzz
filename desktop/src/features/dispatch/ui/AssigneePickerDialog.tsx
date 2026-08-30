import * as React from "react";

import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import {
  useFlattenedUserSearchResults,
  useInfiniteUserSearchQuery,
  useUserSearchFetchMoreOnScroll,
} from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { rankUserCandidatesBySearch } from "@/features/profile/lib/userCandidateSearch";
import type { UserSearchResult } from "@/shared/api/types";
import { parsePubkeyInput } from "@/shared/lib/nostrUtils";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  formatUserSearchCandidateName,
  UserSearchDialog,
  UserSearchResultRow,
  UserSearchResultsPanel,
  UserSearchSectionTitle,
} from "@/shared/ui/user-search-dialog";

const ASSIGNEE_RESULT_LIMIT = 50;

/** Present a pubkey we already hold a profile for as a search candidate. */
function candidateFromProfiles(
  pubkey: string,
  profiles: UserProfileLookup | undefined,
): UserSearchResult {
  const normalized = normalizePubkey(pubkey);
  const profile = profiles?.[normalized];
  return {
    pubkey: normalized,
    displayName: profile?.displayName ?? profile?.name ?? null,
    avatarUrl: profile?.avatarUrl ?? null,
    nip05Handle: profile?.nip05Handle ?? null,
    ownerPubkey: profile?.ownerPubkey ?? null,
    isAgent: profile?.isAgent ?? false,
  };
}

/**
 * Set an issue's assignee by searching people and agents.
 *
 * This is the channel members popup with one section swapped: same shell, same
 * search field, same rows, from `@/shared/ui/user-search-dialog`. The semantics
 * differ only at selection — single-select, replacing the current assignee and
 * closing, where the members popup adds repeatedly and stays open.
 *
 * The candidate list is the community user directory (`search_users` over
 * kind:0), not the issue's channel: `homeChannelId` is nullable and null on
 * most issues, so a channel-scoped list would be empty exactly when the user
 * needs it.
 */
export function AssigneePickerDialog({
  assignee,
  disabled,
  onAssign,
  onOpenChange,
  open,
  profiles,
}: {
  assignee: string | null;
  disabled: boolean;
  onAssign: (pubkey: string | null) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  profiles?: UserProfileLookup;
}) {
  const [searchQuery, setSearchQuery] = React.useState("");
  const deferredSearchQuery = React.useDeferredValue(searchQuery.trim());
  const isArchivedIdentity = useIsArchivedPredicate();
  const normalizedAssignee = assignee ? normalizePubkey(assignee) : null;

  React.useEffect(() => {
    if (!open) setSearchQuery("");
  }, [open]);

  const directoryQuery = useInfiniteUserSearchQuery(deferredSearchQuery, {
    allowEmpty: true,
    enabled: open,
    limit: ASSIGNEE_RESULT_LIMIT,
  });
  const directoryResults = useFlattenedUserSearchResults(directoryQuery.data);
  const handleDirectoryScroll = useUserSearchFetchMoreOnScroll(
    directoryQuery,
    open,
  );

  const candidates = React.useMemo(() => {
    const byPubkey = new Map<string, UserSearchResult>();
    for (const user of directoryResults) {
      const pubkey = normalizePubkey(user.pubkey);
      if (pubkey === normalizedAssignee || isArchivedIdentity(pubkey)) continue;
      if (!byPubkey.has(pubkey)) byPubkey.set(pubkey, { ...user, pubkey });
    }

    const ranked = rankUserCandidatesBySearch({
      allowEmptyQuery: true,
      candidates: [...byPubkey.values()],
      getLabel: formatUserSearchCandidateName,
      limit: Math.max(ASSIGNEE_RESULT_LIMIT, byPubkey.size),
      query: deferredSearchQuery,
    });

    // A pasted hex/npub has to stay assignable. The directory is a full-text
    // search over kind:0 *content*, where a pubkey never appears, so pasting a
    // key returns nothing — and an identity that has published no profile at
    // all is unreachable by name by definition. Offering the parsed key as its
    // own row keeps the capability the raw pubkey field used to provide.
    const pasted = parsePubkeyInput(deferredSearchQuery);
    if (pasted && pasted !== normalizedAssignee && !byPubkey.has(pasted)) {
      ranked.unshift(candidateFromProfiles(pasted, profiles));
    }

    return ranked;
  }, [
    deferredSearchQuery,
    directoryResults,
    isArchivedIdentity,
    normalizedAssignee,
    profiles,
  ]);

  const isSearching =
    directoryQuery.isLoading || searchQuery.trim() !== deferredSearchQuery;

  function select(pubkey: string | null) {
    onAssign(pubkey);
    onOpenChange(false);
  }

  return (
    <UserSearchDialog
      inputDisabled={disabled}
      inputId="dispatch-assignee-search"
      inputTestId="dispatch-assignee-search"
      onOpenChange={onOpenChange}
      onSearchChange={setSearchQuery}
      onSearchKeyDown={(event) => {
        if (event.key !== "Enter" || candidates.length === 0) return;

        event.preventDefault();
        select(candidates[0].pubkey);
      }}
      open={open}
      searchPlaceholder="Search people and agents"
      searchValue={searchQuery}
      testId="dispatch-assignee-picker"
      title="Set assignee"
    >
      <UserSearchResultsPanel
        onScroll={handleDirectoryScroll}
        testId="dispatch-assignee-picker-people"
      >
        {normalizedAssignee ? (
          <>
            <UserSearchSectionTitle>Assigned</UserSearchSectionTitle>
            <UserSearchResultRow
              actionLabel="Unassign"
              disabled={disabled}
              onSelect={() => select(null)}
              testId={`dispatch-assignee-option-${normalizedAssignee}`}
              user={candidateFromProfiles(normalizedAssignee, profiles)}
            />
          </>
        ) : null}
        <UserSearchSectionTitle>People</UserSearchSectionTitle>
        {candidates.map((user) => (
          <UserSearchResultRow
            actionLabel="Assign"
            disabled={disabled}
            key={user.pubkey}
            onSelect={(selected) => select(selected.pubkey)}
            testId={`dispatch-assignee-option-${user.pubkey}`}
            user={user}
          />
        ))}
        {isSearching ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">
            Searching...
          </p>
        ) : null}
        {!isSearching && candidates.length === 0 ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">
            No matching people or agents.
          </p>
        ) : null}
      </UserSearchResultsPanel>
    </UserSearchDialog>
  );
}
