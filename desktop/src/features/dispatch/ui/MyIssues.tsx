import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { selectMyIssues } from "../selectors.mjs";
import type { DispatchIssue, DispatchProject } from "../types";
import {
  DispatchIssueEmpty,
  DispatchIssueList,
  DispatchIssueListSkeleton,
} from "./DispatchIssueList";

export function MyIssues({
  channels,
  currentPubkey,
  isLoading,
  issues,
  onOpenIssue,
  profiles,
  projects,
}: {
  channels: readonly Channel[];
  currentPubkey: string | null | undefined;
  isLoading: boolean;
  issues: readonly DispatchIssue[];
  onOpenIssue: (issueId: string) => void;
  profiles?: UserProfileLookup;
  projects: readonly DispatchProject[];
}) {
  if (isLoading) return <DispatchIssueListSkeleton />;
  const selected = selectMyIssues(issues, currentPubkey);
  if (!selected.length) {
    return (
      <DispatchIssueEmpty
        description="Non-terminal issues assigned to your Desktop identity will appear here."
        title="No active issues assigned to you"
      />
    );
  }
  return (
    <div data-testid="dispatch-my-issues">
      <h2 className="px-5 pb-2 pt-5 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
        My issues · {selected.length}
      </h2>
      <DispatchIssueList
        channels={channels}
        issues={selected}
        onOpenIssue={onOpenIssue}
        profiles={profiles}
        projects={projects}
      />
    </div>
  );
}
