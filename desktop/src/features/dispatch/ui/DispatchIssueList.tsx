import { useIssueAlias } from "./DispatchAliases";
import { Clock3, Hash, Inbox, UserRound } from "lucide-react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { Skeleton } from "@/shared/ui/skeleton";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import type { DispatchIssue, DispatchProject } from "../types";

function triageAge(enteredAt: number, now = Math.floor(Date.now() / 1_000)) {
  const seconds = Math.max(0, now - enteredAt);
  if (seconds < 3_600) return `${Math.max(1, Math.floor(seconds / 60))}m`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3_600)}h`;
  return `${Math.floor(seconds / 86_400)}d`;
}

function PriorityMark({ priority }: { priority: DispatchIssue["priority"] }) {
  if (priority === "P0") {
    return (
      <span
        aria-label="P0 priority"
        className="flex h-4 w-4 shrink-0 items-center justify-center rounded-[4px] bg-destructive text-3xs font-bold text-destructive-foreground"
        role="img"
      >
        !
      </span>
    );
  }
  const filled = priority ? Math.max(1, 4 - Number(priority.slice(1))) : 0;
  return (
    <span
      aria-label={priority ? `${priority} priority` : "No priority"}
      className="flex h-4 w-4 shrink-0 items-end gap-px"
      role="img"
    >
      {[1, 2, 3].map((bar) => (
        <span
          className={
            bar <= filled
              ? "w-[3px] rounded-sm bg-muted-foreground"
              : "w-[3px] rounded-sm bg-muted"
          }
          key={bar}
          style={{ height: `${bar * 4}px` }}
        />
      ))}
    </span>
  );
}

function AssigneeAvatar({
  issue,
  profiles,
}: {
  issue: DispatchIssue;
  profiles?: UserProfileLookup;
}) {
  if (!issue.assignee) {
    return (
      <span
        aria-label="Unassigned"
        className="flex h-6 w-6 items-center justify-center rounded-full border border-dashed border-muted-foreground/70 text-muted-foreground"
        role="img"
      >
        <UserRound aria-hidden className="h-3.5 w-3.5" />
      </span>
    );
  }
  const profile = profiles?.[issue.assignee.toLowerCase()];
  const label = resolveUserLabel({ pubkey: issue.assignee, profiles });
  return (
    // The name only exists on hover here — the row shows the avatar alone.
    <span className="inline-flex" title={label}>
      <UserAvatar
        avatarUrl={profile?.avatarUrl ?? null}
        displayName={label}
        size="sm"
      />
    </span>
  );
}

export function DispatchIssueListSkeleton({ rows = 4 }: { rows?: number }) {
  return (
    <div
      aria-label="Loading issues"
      className="divide-y divide-border"
      role="status"
    >
      {Array.from(
        { length: rows },
        (_, index) => `issue-skeleton-${index}`,
      ).map((key) => (
        <div className="flex h-11 items-center gap-3 px-5" key={key}>
          <Skeleton className="h-4 w-4" />
          <Skeleton className="h-4 w-20" />
          <Skeleton className="h-4 max-w-xl flex-1" />
          <Skeleton className="h-5 w-36" />
        </div>
      ))}
    </div>
  );
}

export function DispatchIssueEmpty({
  description,
  title,
}: {
  description: string;
  title: string;
}) {
  return (
    <div className="flex min-h-40 flex-col items-center justify-center border-y border-dashed border-border px-6 text-center">
      <Inbox aria-hidden className="mb-3 h-5 w-5 text-muted-foreground" />
      <p className="font-medium">{title}</p>
      <p className="mt-1 max-w-md text-sm text-muted-foreground">
        {description}
      </p>
    </div>
  );
}

export function DispatchIssueList({
  channels,
  issues,
  now,
  onOpenIssue,
  profiles,
  projects,
  showTriageAge = false,
}: {
  channels: readonly Channel[];
  issues: readonly DispatchIssue[];
  now?: number;
  onOpenIssue: (issueId: string) => void;
  profiles?: UserProfileLookup;
  projects: readonly DispatchProject[];
  showTriageAge?: boolean;
}) {
  return (
    <div className="divide-y divide-border border-y border-border">
      {issues.map((issue) => {
        const channel = channels.find(
          (candidate) => candidate.id === issue.homeChannelId,
        );
        const project = projects.find(
          (candidate) => candidate.projectId === issue.projectId,
        );
        return (
          <button
            className="flex min-h-11 w-full items-center gap-3 px-5 py-2 text-left transition-colors hover:bg-muted/35 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
            data-testid={`dispatch-issue-${issue.issueId}`}
            key={issue.issueId}
            onClick={() => onOpenIssue(issue.issueId)}
            type="button"
          >
            <PriorityMark priority={issue.priority} />
            <span
              className="w-20 shrink-0 truncate font-mono text-xs text-muted-foreground"
              title={issue.identifier}
            >
              {<DispatchRowAlias issue={issue} />}
            </span>
            <span className="flex min-w-0 flex-1 items-center gap-2">
              <span className="truncate text-sm font-medium">
                {issue.title}
              </span>
              {issue.labels.map((label) => (
                <span
                  className="shrink-0 rounded-md bg-muted px-2 py-0.5 text-xs text-muted-foreground"
                  key={label}
                >
                  {label}
                </span>
              ))}
              {project ? (
                <span
                  className="shrink-0 rounded-md border border-border px-2 py-0.5 text-xs text-muted-foreground"
                  title={`Project ${project.name}`}
                >
                  {project.name}
                </span>
              ) : null}
            </span>
            <span className="ml-auto flex shrink-0 items-center gap-2 text-xs text-muted-foreground">
              {issue.homeChannelId ? (
                <span
                  className="inline-flex max-w-36 items-center gap-1 truncate rounded-md bg-muted px-2 py-0.5"
                  title={channel?.name ?? issue.homeChannelId}
                >
                  <Hash aria-hidden className="h-3 w-3" />
                  <span className="truncate">
                    {channel?.name ?? issue.homeChannelId}
                  </span>
                </span>
              ) : null}
              {showTriageAge ? (
                <span
                  className="inline-flex items-center gap-1"
                  title={new Date(
                    issue.enteredStatusAt * 1_000,
                  ).toLocaleString()}
                >
                  <Clock3 aria-hidden className="h-3.5 w-3.5" /> in triage{" "}
                  {triageAge(issue.enteredStatusAt, now)}
                </span>
              ) : issue.due ? (
                <span>
                  Due {new Date(issue.due.value * 1_000).toLocaleDateString()}
                </span>
              ) : null}
              <AssigneeAvatar issue={issue} profiles={profiles} />
            </span>
          </button>
        );
      })}
    </div>
  );
}

function DispatchRowAlias({ issue }: { issue: DispatchIssue }) {
  return useIssueAlias(issue);
}
