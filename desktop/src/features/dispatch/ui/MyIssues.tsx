import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { selectMyIssues } from "../selectors.mjs";
import type {
  DispatchIssue,
  DispatchProject,
  DispatchLinkedTask,
} from "../types";
import {
  DispatchIssueEmpty,
  DispatchIssueList,
  DispatchIssueListSkeleton,
} from "./DispatchIssueList";

export function MyIssues({
  channels,
  tasks = [],
  currentPubkey,
  isLoading,
  issues,
  onOpenIssue,
  profiles,
  projects,
}: {
  channels: readonly Channel[];
  tasks?: readonly DispatchLinkedTask[];
  currentPubkey: string | null | undefined;
  isLoading: boolean;
  issues: readonly DispatchIssue[];
  onOpenIssue: (issueId: string) => void;
  profiles?: UserProfileLookup;
  projects: readonly DispatchProject[];
}) {
  if (isLoading) return <DispatchIssueListSkeleton />;
  const selected = selectMyIssues(issues, currentPubkey, tasks);
  if (!selected.length) {
    return (
      <DispatchIssueEmpty
        description="Active issues you own or delegated appear here."
        title="No active work for you"
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
