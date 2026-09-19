import { Info, TriangleAlert } from "lucide-react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import { splitTriageIssues } from "../selectors.mjs";
import type { DispatchIssue, DispatchProject } from "../types";
import {
  DispatchIssueEmpty,
  DispatchIssueList,
  DispatchIssueListSkeleton,
} from "./DispatchIssueList";

export function TriageInbox({
  channels,
  error,
  isLoading,
  issues,
  onOpenIssue,
  onRetry,
  profiles,
  projects,
}: {
  channels: readonly Channel[];
  error: Error | null;
  isLoading: boolean;
  issues: readonly DispatchIssue[];
  onOpenIssue: (issueId: string) => void;
  onRetry: () => void;
  profiles?: UserProfileLookup;
  projects: readonly DispatchProject[];
}) {
  if (isLoading) return <DispatchIssueListSkeleton />;
  if (error) {
    return (
      <Alert className="m-5" variant="destructive">
        <TriangleAlert aria-hidden className="h-4 w-4" />
        <AlertTitle>Triage is unavailable</AlertTitle>
        <AlertDescription>
          Canonical root or history data could not be loaded. Buzz will not show
          a stale inbox.
          <span className="mt-2 block text-xs">{error.message}</span>
          <Button
            className="mt-3"
            onClick={onRetry}
            size="sm"
            type="button"
            variant="outline"
          >
            Retry
          </Button>
        </AlertDescription>
      </Alert>
    );
  }
  const sections = splitTriageIssues(issues);
  const total = sections.unassigned.length + sections.assigned.length;
  if (total === 0) {
    return (
      <DispatchIssueEmpty
        description="New issues will appear here from the current workspace fold."
        title="Triage is clear"
      />
    );
  }
  return (
    <div data-testid="dispatch-triage">
      <section aria-labelledby="dispatch-unassigned-heading">
        <h2
          className="px-5 pb-2 pt-5 text-xs font-semibold uppercase tracking-wider text-muted-foreground"
          id="dispatch-unassigned-heading"
        >
          Unassigned · {sections.unassigned.length}
        </h2>
        {sections.unassigned.length ? (
          <DispatchIssueList
            channels={channels}
            issues={sections.unassigned}
            onOpenIssue={onOpenIssue}
            profiles={profiles}
            projects={projects}
            showTriageAge
          />
        ) : (
          <p className="border-y border-border px-5 py-4 text-sm text-muted-foreground">
            No unassigned triage issues.
          </p>
        )}
      </section>
      <section aria-labelledby="dispatch-assigned-heading">
        <h2
          className="px-5 pb-2 pt-6 text-xs font-semibold uppercase tracking-wider text-muted-foreground"
          id="dispatch-assigned-heading"
        >
          Assigned · {sections.assigned.length}
        </h2>
        {sections.assigned.length ? (
          <DispatchIssueList
            channels={channels}
            issues={sections.assigned}
            onOpenIssue={onOpenIssue}
            profiles={profiles}
            projects={projects}
            showTriageAge
          />
        ) : (
          <p className="border-y border-border px-5 py-4 text-sm text-muted-foreground">
            No assigned triage issues.
          </p>
        )}
      </section>
      <div className="m-5 flex items-center gap-2 rounded-lg border border-dashed border-border px-4 py-3 text-sm text-muted-foreground">
        <Info aria-hidden className="h-4 w-4 shrink-0" />
        Choose an owner and priority here. Open an issue to review its details
        and assign the work.
      </div>
    </div>
  );
}
