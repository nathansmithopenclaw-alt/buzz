import type {
  DispatchIssue,
  DispatchPriority,
  DispatchProject,
  DispatchStatus,
} from "./types";
import { truncatePubkey } from "@/shared/lib/pubkey";

export type DispatchGroupBy = "status" | "priority" | "assignee" | "project";

export type DispatchFilters = {
  status: DispatchStatus | "all";
  priority: DispatchPriority | "none" | "all";
  assignee: string;
  project: string;
  label: string;
  query: string;
};

export type DispatchSortKey =
  | "identifier"
  | "title"
  | "status"
  | "priority"
  | "assignee"
  | "project"
  | "due";

export type DispatchSort = {
  key: DispatchSortKey;
  direction: "asc" | "desc";
};

export type DispatchProfile = {
  displayName: string | null;
  name?: string | null;
  avatarUrl: string | null;
};

export type DispatchProfileMap = Record<string, DispatchProfile>;

export const EMPTY_DISPATCH_FILTERS: DispatchFilters = {
  status: "all",
  priority: "all",
  assignee: "all",
  project: "all",
  label: "all",
  query: "",
};

export const STATUS_ORDER: DispatchStatus[] = [
  "triage",
  "backlog",
  "todo",
  "in_progress",
  "in_review",
  "done",
  "cancelled",
];

const PRIORITY_ORDER: Array<DispatchPriority | "none"> = [
  "P0",
  "P1",
  "P2",
  "P3",
  "none",
];

export function statusLabel(status: DispatchStatus): string {
  return status
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export function shortPubkey(pubkey: string): string {
  return truncatePubkey(pubkey);
}

export function assigneeLabel(
  pubkey: string | null,
  profiles: DispatchProfileMap,
): string {
  if (!pubkey) return "None";
  const profile = profiles[pubkey.toLowerCase()];
  return (
    profile?.displayName?.trim() || profile?.name?.trim() || shortPubkey(pubkey)
  );
}

export function projectLabel(
  projectId: string | null,
  projects: DispatchProject[],
): string {
  if (!projectId) return "None";
  return (
    projects.find((project) => project.projectId === projectId)?.name ??
    projectId
  );
}

export function filterDispatchIssues(
  issues: DispatchIssue[],
  filters: DispatchFilters,
): DispatchIssue[] {
  const query = filters.query.trim().toLocaleLowerCase();
  return issues.filter((issue) => {
    if (filters.status !== "all" && issue.status !== filters.status)
      return false;
    if (
      filters.priority !== "all" &&
      (issue.priority ?? "none") !== filters.priority
    )
      return false;
    if (
      filters.assignee !== "all" &&
      (issue.assignee ?? "none") !== filters.assignee
    )
      return false;
    if (
      filters.project !== "all" &&
      (issue.projectId ?? "none") !== filters.project
    )
      return false;
    if (filters.label !== "all" && !issue.labels.includes(filters.label))
      return false;
    if (!query) return true;
    return [issue.identifier, issue.title, issue.body, ...issue.labels].some(
      (value) => value.toLocaleLowerCase().includes(query),
    );
  });
}

export type DispatchIssueGroup = {
  id: string;
  label: string;
  issues: DispatchIssue[];
};

export function groupDispatchIssues(
  issues: DispatchIssue[],
  groupBy: DispatchGroupBy,
  projects: DispatchProject[],
  profiles: DispatchProfileMap,
): DispatchIssueGroup[] {
  const byKey = new Map<string, DispatchIssue[]>();
  for (const issue of issues) {
    const key =
      groupBy === "status"
        ? issue.status
        : groupBy === "priority"
          ? (issue.priority ?? "none")
          : groupBy === "assignee"
            ? (issue.assignee ?? "none")
            : (issue.projectId ?? "none");
    const group = byKey.get(key) ?? [];
    group.push(issue);
    byKey.set(key, group);
  }
  const keys = [...byKey.keys()].sort((left, right) => {
    if (groupBy === "status")
      return (
        STATUS_ORDER.indexOf(left as DispatchStatus) -
        STATUS_ORDER.indexOf(right as DispatchStatus)
      );
    if (groupBy === "priority")
      return (
        PRIORITY_ORDER.indexOf(left as DispatchPriority | "none") -
        PRIORITY_ORDER.indexOf(right as DispatchPriority | "none")
      );
    const leftLabel =
      groupBy === "assignee"
        ? assigneeLabel(left === "none" ? null : left, profiles)
        : projectLabel(left === "none" ? null : left, projects);
    const rightLabel =
      groupBy === "assignee"
        ? assigneeLabel(right === "none" ? null : right, profiles)
        : projectLabel(right === "none" ? null : right, projects);
    return leftLabel.localeCompare(rightLabel);
  });
  return keys.map((key) => ({
    id: key,
    label:
      groupBy === "status"
        ? statusLabel(key as DispatchStatus)
        : groupBy === "priority"
          ? key === "none"
            ? "None"
            : key
          : groupBy === "assignee"
            ? assigneeLabel(key === "none" ? null : key, profiles)
            : projectLabel(key === "none" ? null : key, projects),
    issues: byKey.get(key) ?? [],
  }));
}

function sortableValue(
  issue: DispatchIssue,
  key: DispatchSortKey,
  projects: DispatchProject[],
  profiles: DispatchProfileMap,
): string | number {
  switch (key) {
    case "identifier":
      return issue.identifier;
    case "title":
      return issue.title;
    case "status":
      return STATUS_ORDER.indexOf(issue.status);
    case "priority":
      return PRIORITY_ORDER.indexOf(issue.priority ?? "none");
    case "assignee":
      return assigneeLabel(issue.assignee, profiles);
    case "project":
      return projectLabel(issue.projectId, projects);
    case "due":
      return issue.due?.value ?? Number.MAX_SAFE_INTEGER;
  }
}

export function sortDispatchIssues(
  issues: DispatchIssue[],
  sort: DispatchSort,
  projects: DispatchProject[],
  profiles: DispatchProfileMap,
): DispatchIssue[] {
  const direction = sort.direction === "asc" ? 1 : -1;
  return [...issues].sort((left, right) => {
    const a = sortableValue(left, sort.key, projects, profiles);
    const b = sortableValue(right, sort.key, projects, profiles);
    const compared =
      typeof a === "number" && typeof b === "number"
        ? a - b
        : String(a).localeCompare(String(b));
    const priorityRecency =
      sort.key === "priority" && compared === 0
        ? right.tipCreatedAt - left.tipCreatedAt
        : 0;
    return (
      compared * direction ||
      priorityRecency ||
      left.identifier.localeCompare(right.identifier)
    );
  });
}

export function sortDispatchBoardIssues(
  issues: DispatchIssue[],
  projects: DispatchProject[],
  profiles: DispatchProfileMap,
): DispatchIssue[] {
  return sortDispatchIssues(
    issues,
    { key: "priority", direction: "asc" },
    projects,
    profiles,
  );
}

export function blockedIssueIds(issues: DispatchIssue[]): Set<string> {
  return new Set(
    issues.flatMap((issue) =>
      issue.relations
        .filter((relation) => relation.kind === "blocks")
        .map((relation) => relation.targetIssueId),
    ),
  );
}
