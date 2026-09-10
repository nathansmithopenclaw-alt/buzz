import { useIssueAlias } from "./DispatchAliases";
import type * as React from "react";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  Ban,
  CheckCircle2,
  Circle,
  CircleDashed,
  CircleDot,
  Clock3,
  Search,
  UserRound,
} from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import type { DispatchIssue, DispatchProject, DispatchStatus } from "../types";
import {
  assigneeLabel,
  blockedIssueIds,
  filterDispatchIssues,
  projectLabel,
  shortPubkey,
  sortDispatchIssues,
  statusLabel,
  type DispatchFilters,
  type DispatchGroupBy,
  type DispatchProfileMap,
  type DispatchSort,
  type DispatchSortKey,
} from "../viewModel";

const selectClassName =
  "h-8 rounded-lg border border-input/40 bg-background px-2 text-xs text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring";

function StatusGlyph({
  status,
  className,
}: {
  status: DispatchStatus;
  className?: string;
}) {
  const classes = cn(
    "h-3.5 w-3.5 shrink-0",
    status === "triage" && "text-[hsl(var(--chart-4))]",
    (status === "backlog" || status === "todo") && "text-muted-foreground",
    status === "in_progress" && "text-warning",
    status === "in_review" && "text-[hsl(var(--chart-5))]",
    status === "done" && "text-primary",
    status === "cancelled" && "text-muted-foreground",
    className,
  );
  if (status === "backlog") return <CircleDashed className={classes} />;
  if (status === "todo") return <Circle className={classes} />;
  if (status === "in_progress") return <CircleDot className={classes} />;
  if (status === "in_review") return <Clock3 className={classes} />;
  if (status === "done") return <CheckCircle2 className={classes} />;
  if (status === "cancelled") return <Ban className={classes} />;
  return <AlertTriangle className={classes} />;
}

function PriorityGlyph({ priority }: { priority: DispatchIssue["priority"] }) {
  if (!priority) return null;
  if (priority === "P0") {
    return (
      <span
        aria-label="Priority P0"
        className="inline-flex h-3.5 w-3.5 items-center justify-center rounded bg-destructive text-3xs font-bold text-destructive-foreground"
        role="img"
      >
        !
      </span>
    );
  }
  const active = priority === "P1" ? 3 : priority === "P2" ? 2 : 1;
  return (
    <span
      aria-label={`Priority ${priority}`}
      className="flex h-3 items-end gap-0.5"
      role="img"
    >
      {[1, 2, 3].map((bar) => (
        <span
          className={cn(
            "w-0.5 rounded-sm bg-muted-foreground/25",
            bar <= active && priority === "P1" && "bg-warning",
            bar <= active && priority === "P2" && "bg-[hsl(var(--chart-4))]",
            bar <= active && priority === "P3" && "bg-muted-foreground",
          )}
          key={bar}
          style={{ height: `${bar * 3 + 2}px` }}
        />
      ))}
    </span>
  );
}

function AssigneeAvatar({
  pubkey,
  profiles,
}: {
  pubkey: string | null;
  profiles: DispatchProfileMap;
}) {
  if (!pubkey) {
    return (
      <span
        aria-label="No assignee"
        className="flex h-[18px] w-[18px] items-center justify-center rounded-full border border-dashed border-muted-foreground/70 text-muted-foreground"
        role="img"
      >
        <UserRound className="h-3 w-3" />
      </span>
    );
  }
  const profile = profiles[pubkey.toLowerCase()];
  const label = assigneeLabel(pubkey, profiles);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <UserAvatar
          accent
          avatarUrl={profile?.avatarUrl ?? null}
          className="h-[18px] w-[18px]"
          displayName={label}
          size="xs"
        />
      </TooltipTrigger>
      <TooltipContent>{profile ? label : shortPubkey(pubkey)}</TooltipContent>
    </Tooltip>
  );
}

function IssueIdentifier({
  issue,
  identifier,
}: {
  issue: DispatchIssue;
  identifier?: string;
}) {
  const alias = useIssueAlias(issue);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          className="block w-full min-w-0 truncate text-2xs text-muted-foreground"
          data-testid="dispatch-issue-identifier"
          title={issue.identifier}
        >
          {identifier ?? alias}
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-sm break-all">
        {issue.identifier}
      </TooltipContent>
    </Tooltip>
  );
}

export function IssueCard({
  issue,
  identifier,
  project,
  profiles,
  blocked,
  onOpenIssue,
  draggable,
  onDragStart,
  onDragEnd,
}: {
  issue: DispatchIssue;
  identifier?: string;
  draggable?: boolean;
  onDragStart?: React.DragEventHandler<HTMLButtonElement>;
  onDragEnd?: React.DragEventHandler<HTMLButtonElement>;
  project: DispatchProject | undefined;
  profiles: DispatchProfileMap;
  blocked: boolean;
  onOpenIssue: (issueId: string) => void;
}) {
  return (
    <button
      className={cn(
        "flex w-full flex-col gap-2 rounded-lg border border-border/55 bg-card p-2.5 text-left shadow-xs transition-colors hover:bg-accent/45 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
        issue.status === "done" && "opacity-60",
      )}
      draggable={draggable}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      data-testid="dispatch-issue-card"
      onClick={() => onOpenIssue(issue.issueId)}
      type="button"
    >
      <span className="flex w-full min-w-0 items-center gap-1.5">
        <StatusGlyph status={issue.status} />
        <IssueIdentifier issue={issue} identifier={identifier} />
        <span className="ml-auto flex shrink-0 items-center gap-1.5">
          {blocked ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <AlertTriangle
                  aria-label="Blocked"
                  className="h-3.5 w-3.5 text-destructive"
                />
              </TooltipTrigger>
              <TooltipContent>Blocked by another issue</TooltipContent>
            </Tooltip>
          ) : null}
          <PriorityGlyph priority={issue.priority} />
        </span>
      </span>
      <span className="line-clamp-2 min-h-8 text-xs leading-4 text-foreground">
        {issue.title}
      </span>
      <span className="flex min-h-[18px] w-full items-center gap-1 overflow-hidden">
        {project ? (
          <span className="max-w-28 truncate rounded bg-muted/70 px-1.5 py-0.5 text-2xs text-muted-foreground">
            {project.name}
          </span>
        ) : null}
        {issue.labels.slice(0, project ? 1 : 2).map((label) => (
          <span
            className="max-w-24 truncate rounded bg-muted/70 px-1.5 py-0.5 text-2xs text-muted-foreground"
            key={label}
          >
            {label}
          </span>
        ))}
        <span className="ml-auto shrink-0">
          <AssigneeAvatar profiles={profiles} pubkey={issue.assignee} />
        </span>
      </span>
    </button>
  );
}

export function DispatchFiltersBar({
  filters,
  issues,
  projects,
  onChange,
  groupBy,
  onGroupByChange,
}: {
  filters: DispatchFilters;
  issues: DispatchIssue[];
  projects: DispatchProject[];
  onChange: (filters: DispatchFilters) => void;
  groupBy?: DispatchGroupBy;
  onGroupByChange?: (groupBy: DispatchGroupBy) => void;
}) {
  const assignees = [
    ...new Set(issues.map((issue) => issue.assignee).filter(Boolean)),
  ] as string[];
  const labels = [...new Set(issues.flatMap((issue) => issue.labels))].sort();
  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-border/45 px-5 py-2">
      <div className="relative min-w-48 flex-1 sm:max-w-72">
        <Search className="pointer-events-none absolute left-2.5 top-2 h-3.5 w-3.5 text-muted-foreground" />
        <Input
          aria-label="Search issues"
          className="h-8 pl-8 text-xs"
          data-testid="dispatch-filter-query"
          onChange={(event) =>
            onChange({ ...filters, query: event.target.value })
          }
          placeholder="Search issues"
          value={filters.query}
        />
      </div>
      <select
        aria-label="Status filter"
        className={selectClassName}
        data-testid="dispatch-filter-status"
        onChange={(event) =>
          onChange({
            ...filters,
            status: event.target.value as DispatchFilters["status"],
          })
        }
        value={filters.status}
      >
        <option value="all">All statuses</option>
        {(
          [
            "triage",
            "backlog",
            "todo",
            "in_progress",
            "in_review",
            "done",
            "cancelled",
          ] as const
        ).map((status) => (
          <option key={status} value={status}>
            {statusLabel(status)}
          </option>
        ))}
      </select>
      <select
        aria-label="Priority filter"
        className={selectClassName}
        data-testid="dispatch-filter-priority"
        onChange={(event) =>
          onChange({
            ...filters,
            priority: event.target.value as DispatchFilters["priority"],
          })
        }
        value={filters.priority}
      >
        <option value="all">All priorities</option>
        <option value="none">No priority</option>
        {(["P0", "P1", "P2", "P3"] as const).map((priority) => (
          <option key={priority} value={priority}>
            {priority}
          </option>
        ))}
      </select>
      <select
        aria-label="Assignee filter"
        className={selectClassName}
        data-testid="dispatch-filter-assignee"
        onChange={(event) =>
          onChange({ ...filters, assignee: event.target.value })
        }
        value={filters.assignee}
      >
        <option value="all">All assignees</option>
        <option value="none">No assignee</option>
        {assignees.map((pubkey) => (
          <option key={pubkey} value={pubkey}>
            {shortPubkey(pubkey)}
          </option>
        ))}
      </select>
      <select
        aria-label="Project filter"
        className={selectClassName}
        data-testid="dispatch-filter-project"
        onChange={(event) =>
          onChange({ ...filters, project: event.target.value })
        }
        value={filters.project}
      >
        <option value="all">All projects</option>
        <option value="none">No project</option>
        {projects.map((project) => (
          <option key={project.projectId} value={project.projectId}>
            {project.name}
          </option>
        ))}
      </select>
      <select
        aria-label="Label filter"
        className={selectClassName}
        data-testid="dispatch-filter-label"
        onChange={(event) =>
          onChange({ ...filters, label: event.target.value })
        }
        value={filters.label}
      >
        <option value="all">All labels</option>
        {labels.map((label) => (
          <option key={label} value={label}>
            {label}
          </option>
        ))}
      </select>
      {groupBy && onGroupByChange ? (
        <select
          aria-label="Group board by"
          className={selectClassName}
          data-testid="dispatch-group-by"
          onChange={(event) =>
            onGroupByChange(event.target.value as DispatchGroupBy)
          }
          value={groupBy}
        >
          <option value="status">Group: Status</option>
          <option value="priority">Group: Priority</option>
          <option value="assignee">Group: Assignee</option>
          <option value="project">Group: Project</option>
        </select>
      ) : null}
    </div>
  );
}

const LIST_COLUMNS: Array<{ key: DispatchSortKey; label: string }> = [
  { key: "identifier", label: "Issue" },
  { key: "title", label: "Title" },
  { key: "status", label: "Status" },
  { key: "priority", label: "Priority" },
  { key: "assignee", label: "Owner" },
  { key: "project", label: "Project" },
  { key: "due", label: "Due" },
];

function formatDue(issue: DispatchIssue): string {
  if (!issue.due) return "—";
  const value =
    issue.due.value < 10_000_000_000
      ? issue.due.value * 1_000
      : issue.due.value;
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(
    value,
  );
}

export function DispatchListPanel({
  excludedIssueIds,
  filters,
  issues,
  onClearOverflow,
  onFiltersChange,
  projects,
  profiles,
  onOpenIssue,
  onSortChange,
  sort,
  selected = [],
  onSelection,
}: {
  excludedIssueIds: string[];
  filters: DispatchFilters;
  issues: DispatchIssue[];
  onClearOverflow: () => void;
  onFiltersChange: (filters: DispatchFilters) => void;
  projects: DispatchProject[];
  profiles: DispatchProfileMap;
  onOpenIssue: (issueId: string) => void;
  onSortChange: (sort: DispatchSort) => void;
  sort: DispatchSort;
  selected?: string[];
  onSelection?: (ids: string[]) => void;
}) {
  const filtered = filterDispatchIssues(issues, filters);
  const excluded = new Set(excludedIssueIds);
  const sorted = sortDispatchIssues(
    filtered.filter((issue) => !excluded.has(issue.issueId)),
    sort,
    projects,
    profiles,
  );
  const blocked = blockedIssueIds(issues);
  const changeSort = (key: DispatchSortKey) =>
    onSortChange({
      key,
      direction: sort.key === key && sort.direction === "asc" ? "desc" : "asc",
    });
  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      data-testid="dispatch-list-mount"
    >
      <DispatchFiltersBar
        filters={filters}
        issues={issues}
        onChange={onFiltersChange}
        projects={projects}
      />
      {excludedIssueIds.length > 0 ? (
        <div
          className="flex items-center justify-between border-b border-border/45 bg-muted/25 px-5 py-2 text-xs text-muted-foreground"
          data-testid="dispatch-overflow-context"
        >
          <span>Showing the additional items from this Board column.</span>
          <button
            className="font-medium text-foreground hover:underline"
            onClick={onClearOverflow}
            type="button"
          >
            Show all matching issues
          </button>
        </div>
      ) : null}
      <div className="min-h-0 flex-1 overflow-auto">
        <table
          className="w-full min-w-[940px] table-fixed border-collapse text-xs"
          data-testid="dispatch-list"
        >
          <thead className="sticky top-0 z-10 bg-background">
            <tr className="border-b border-border/55 text-left text-muted-foreground">
              {onSelection ? (
                <th className="w-10 px-3">
                  <input
                    type="checkbox"
                    aria-label="Select up to 50 matching issues"
                    checked={
                      sorted.length > 0 &&
                      sorted
                        .slice(0, 50)
                        .every((issue) => selected.includes(issue.issueId))
                    }
                    onChange={(event) =>
                      onSelection(
                        event.target.checked
                          ? sorted.slice(0, 50).map((issue) => issue.issueId)
                          : [],
                      )
                    }
                  />
                </th>
              ) : null}
              {LIST_COLUMNS.map((column) => (
                <th className="px-3 py-2 font-medium" key={column.key}>
                  <button
                    className="flex items-center gap-1 hover:text-foreground"
                    data-testid={`dispatch-sort-${column.key}`}
                    onClick={() => changeSort(column.key)}
                    type="button"
                  >
                    {column.label}
                    {sort.key === column.key ? (
                      sort.direction === "asc" ? (
                        <ArrowUp className="h-3 w-3" />
                      ) : (
                        <ArrowDown className="h-3 w-3" />
                      )
                    ) : null}
                  </button>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {sorted.map((issue) => (
              <tr
                className={cn(
                  "cursor-pointer border-b border-border/35 hover:bg-accent/35",
                  issue.status === "done" && "opacity-65",
                )}
                data-testid="dispatch-list-row"
                key={issue.issueId}
                onClick={() => onOpenIssue(issue.issueId)}
              >
                {onSelection ? (
                  <td className="w-10 px-3">
                    <input
                      type="checkbox"
                      aria-label={`Select ${issue.title}`}
                      checked={selected.includes(issue.issueId)}
                      disabled={
                        selected.length >= 50 &&
                        !selected.includes(issue.issueId)
                      }
                      onClick={(event) => event.stopPropagation()}
                      onChange={(event) =>
                        onSelection(
                          event.target.checked
                            ? [...selected, issue.issueId]
                            : selected.filter((id) => id !== issue.issueId),
                        )
                      }
                    />
                  </td>
                ) : null}
                <td className="w-28 max-w-28 px-3 py-2">
                  <IssueIdentifier issue={issue} />
                </td>
                <td
                  className="max-w-sm truncate px-3 py-2 font-medium"
                  title={issue.title}
                >
                  <button
                    type="button"
                    className="w-full truncate text-left focus-visible:underline"
                    onClick={(event) => {
                      event.stopPropagation();
                      onOpenIssue(issue.issueId);
                    }}
                  >
                    {issue.title}
                  </button>
                </td>
                <td className="px-3 py-2">
                  <span className="flex items-center gap-1.5">
                    <StatusGlyph status={issue.status} />
                    {statusLabel(issue.status)}
                  </span>
                </td>
                <td className="px-3 py-2">
                  <span className="flex items-center gap-1.5">
                    <PriorityGlyph priority={issue.priority} />
                    {issue.priority ?? "None"}
                  </span>
                </td>
                <td className="max-w-40 px-3 py-2">
                  <span className="flex items-center gap-1.5">
                    <AssigneeAvatar
                      profiles={profiles}
                      pubkey={issue.assignee}
                    />
                    <span className="truncate">
                      {assigneeLabel(issue.assignee, profiles)}
                    </span>
                  </span>
                </td>
                <td className="max-w-40 truncate px-3 py-2">
                  {projectLabel(issue.projectId, projects)}
                </td>
                <td className="whitespace-nowrap px-3 py-2">
                  {blocked.has(issue.issueId) ? (
                    <AlertTriangle
                      aria-label="Blocked"
                      className="mr-1 inline h-3.5 w-3.5 text-destructive"
                    />
                  ) : null}
                  {formatDue(issue)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {sorted.length === 0 ? (
          <div className="p-10 text-center text-sm text-muted-foreground">
            No issues match these filters.
          </div>
        ) : null}
      </div>
    </div>
  );
}

export function DispatchPanelsTooltipProvider({
  children,
}: React.PropsWithChildren) {
  return <TooltipProvider delayDuration={180}>{children}</TooltipProvider>;
}
