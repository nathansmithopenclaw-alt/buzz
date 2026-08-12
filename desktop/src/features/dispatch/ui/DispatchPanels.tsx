import * as React from "react";
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
import { Avatar, AvatarFallback, AvatarImage } from "@/shared/ui/avatar";
import { Input } from "@/shared/ui/input";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";
import type { DispatchIssue, DispatchProject, DispatchStatus } from "../types";
import {
  assigneeLabel,
  blockedIssueIds,
  EMPTY_DISPATCH_FILTERS,
  filterDispatchIssues,
  groupDispatchIssues,
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

const BOARD_STATUSES: DispatchStatus[] = [
  "triage",
  "backlog",
  "in_progress",
  "in_review",
  "done",
];
const CARD_LIMIT = 3;

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
        <Avatar className="h-[18px] w-[18px]">
          {profile?.avatarUrl ? (
            <AvatarImage alt="" src={profile.avatarUrl} />
          ) : null}
          <AvatarFallback className="bg-primary text-3xs font-semibold text-primary-foreground">
            {label.charAt(0).toUpperCase()}
          </AvatarFallback>
        </Avatar>
      </TooltipTrigger>
      <TooltipContent>{profile ? label : shortPubkey(pubkey)}</TooltipContent>
    </Tooltip>
  );
}

function IssueIdentifier({ issue }: { issue: DispatchIssue }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          className="block w-full min-w-0 truncate text-2xs text-muted-foreground"
          data-testid="dispatch-issue-identifier"
          title={issue.identifier}
        >
          {issue.identifier}
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-sm break-all">
        {issue.identifier}
      </TooltipContent>
    </Tooltip>
  );
}

function IssueCard({
  issue,
  project,
  profiles,
  blocked,
  onOpenIssue,
}: {
  issue: DispatchIssue;
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
      data-testid="dispatch-issue-card"
      onClick={() => onOpenIssue(issue.issueId)}
      type="button"
    >
      <span className="flex w-full min-w-0 items-center gap-1.5">
        <StatusGlyph status={issue.status} />
        <IssueIdentifier issue={issue} />
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

function DispatchFiltersBar({
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

export function DispatchBoardPanel({
  issues,
  projects,
  profiles,
  onOpenIssue,
}: {
  issues: DispatchIssue[];
  projects: DispatchProject[];
  profiles: DispatchProfileMap;
  onOpenIssue: (issueId: string) => void;
}) {
  const [filters, setFilters] = React.useState(EMPTY_DISPATCH_FILTERS);
  const [groupBy, setGroupBy] = React.useState<DispatchGroupBy>("status");
  const filtered = filterDispatchIssues(issues, filters);
  const grouped = groupDispatchIssues(filtered, groupBy, projects, profiles);
  const groups =
    groupBy === "status"
      ? BOARD_STATUSES.map(
          (status) =>
            grouped.find((group) => group.id === status) ?? {
              id: status,
              label: statusLabel(status),
              issues: [],
            },
        )
      : grouped;
  const blocked = blockedIssueIds(issues);
  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      data-testid="dispatch-board-mount"
    >
      <DispatchFiltersBar
        filters={filters}
        groupBy={groupBy}
        issues={issues}
        onChange={setFilters}
        onGroupByChange={setGroupBy}
        projects={projects}
      />
      <div
        className="flex min-h-0 flex-1 items-start gap-3 overflow-x-auto p-4"
        data-testid="dispatch-board"
      >
        {groups.map((group) => (
          <section
            className="w-[262px] shrink-0"
            data-testid={`dispatch-group-${group.id}`}
            key={group.id}
          >
            <header className="flex items-center gap-2 px-1.5 pb-2.5 text-xs font-semibold">
              {groupBy === "status" ? (
                <StatusGlyph status={group.id as DispatchStatus} />
              ) : null}
              <span className="truncate">{group.label}</span>
              <span className="font-normal text-muted-foreground">
                {group.issues.length}
              </span>
            </header>
            <div className="flex flex-col gap-2">
              {group.issues.slice(0, CARD_LIMIT).map((issue) => (
                <IssueCard
                  blocked={blocked.has(issue.issueId)}
                  issue={issue}
                  key={issue.issueId}
                  onOpenIssue={onOpenIssue}
                  profiles={profiles}
                  project={projects.find(
                    (project) => project.projectId === issue.projectId,
                  )}
                />
              ))}
              {group.issues.length > CARD_LIMIT ? (
                <div className="py-1 text-center text-2xs text-muted-foreground">
                  + {group.issues.length - CARD_LIMIT} more
                </div>
              ) : null}
            </div>
          </section>
        ))}
        {groups.length === 0 ? (
          <div className="m-auto text-sm text-muted-foreground">
            No issues match these filters.
          </div>
        ) : null}
      </div>
    </div>
  );
}

const LIST_COLUMNS: Array<{ key: DispatchSortKey; label: string }> = [
  { key: "identifier", label: "Issue" },
  { key: "title", label: "Title" },
  { key: "status", label: "Status" },
  { key: "priority", label: "Priority" },
  { key: "assignee", label: "Assignee" },
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
  issues,
  projects,
  profiles,
  onOpenIssue,
}: {
  issues: DispatchIssue[];
  projects: DispatchProject[];
  profiles: DispatchProfileMap;
  onOpenIssue: (issueId: string) => void;
}) {
  const [filters, setFilters] = React.useState(EMPTY_DISPATCH_FILTERS);
  const [sort, setSort] = React.useState<DispatchSort>({
    key: "identifier",
    direction: "asc",
  });
  const filtered = filterDispatchIssues(issues, filters);
  const sorted = sortDispatchIssues(filtered, sort, projects, profiles);
  const blocked = blockedIssueIds(issues);
  const changeSort = (key: DispatchSortKey) =>
    setSort((current) => ({
      key,
      direction:
        current.key === key && current.direction === "asc" ? "desc" : "asc",
    }));
  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      data-testid="dispatch-list-mount"
    >
      <DispatchFiltersBar
        filters={filters}
        issues={issues}
        onChange={setFilters}
        projects={projects}
      />
      <div className="min-h-0 flex-1 overflow-auto">
        <table
          className="w-full min-w-[940px] table-fixed border-collapse text-xs"
          data-testid="dispatch-list"
        >
          <thead className="sticky top-0 z-10 bg-background">
            <tr className="border-b border-border/55 text-left text-muted-foreground">
              {LIST_COLUMNS.map((column) => (
                <th className="px-3 py-2 font-medium" key={column.key}>
                  <button
                    className="flex items-center gap-1 hover:text-foreground"
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
                <td className="w-40 max-w-40 px-3 py-2">
                  <IssueIdentifier issue={issue} />
                </td>
                <td
                  className="max-w-sm truncate px-3 py-2 font-medium"
                  title={issue.title}
                >
                  {issue.title}
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
