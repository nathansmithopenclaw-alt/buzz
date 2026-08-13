import type { DispatchPriority, DispatchStatus } from "./types";
import type {
  DispatchFilters,
  DispatchGroupBy,
  DispatchSort,
  DispatchSortKey,
} from "./viewModel";
import { EMPTY_DISPATCH_FILTERS } from "./viewModel";

export type DispatchView = "board" | "list" | "triage" | "my-issues";

export type DispatchRouteSearch = {
  workspace?: string;
  view?: DispatchView;
  status?: DispatchStatus | "all";
  priority?: DispatchPriority | "none" | "all";
  assignee?: string;
  project?: string;
  label?: string;
  query?: string;
  group?: DispatchGroupBy;
  sort?: DispatchSortKey;
  direction?: DispatchSort["direction"];
  exclude?: string;
};

export const DISPATCH_SEARCH_KEYS = [
  "workspace",
  "view",
  "status",
  "priority",
  "assignee",
  "project",
  "label",
  "query",
  "group",
  "sort",
  "direction",
  "exclude",
] as const;

const VIEWS: DispatchView[] = ["board", "list", "triage", "my-issues"];
const STATUSES: Array<DispatchStatus | "all"> = [
  "all",
  "triage",
  "backlog",
  "todo",
  "in_progress",
  "in_review",
  "done",
  "cancelled",
];
const PRIORITIES: Array<DispatchPriority | "none" | "all"> = [
  "all",
  "none",
  "P0",
  "P1",
  "P2",
  "P3",
];
const GROUPS: DispatchGroupBy[] = ["status", "priority", "assignee", "project"];
const SORTS: DispatchSortKey[] = [
  "identifier",
  "title",
  "status",
  "priority",
  "assignee",
  "project",
  "due",
];
const DIRECTIONS: DispatchSort["direction"][] = ["asc", "desc"];
const ISSUE_ID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function nonEmptyString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function member<T extends string>(
  value: unknown,
  values: readonly T[],
): T | undefined {
  return typeof value === "string" && values.includes(value as T)
    ? (value as T)
    : undefined;
}

export function parseExcludedIssueIds(value: unknown): string[] {
  if (typeof value !== "string") return [];
  return [...new Set(value.split(",").filter((id) => ISSUE_ID.test(id)))];
}

export function encodeExcludedIssueIds(issueIds: readonly string[]): string {
  return issueIds.filter((id) => ISSUE_ID.test(id)).join(",");
}

export function validateDispatchSearch(
  search: Record<string, unknown>,
): DispatchRouteSearch {
  const exclude = parseExcludedIssueIds(search.exclude);
  return {
    workspace: nonEmptyString(search.workspace),
    view: member(search.view, VIEWS),
    status: member(search.status, STATUSES),
    priority: member(search.priority, PRIORITIES),
    assignee: nonEmptyString(search.assignee),
    project: nonEmptyString(search.project),
    label: nonEmptyString(search.label),
    query: nonEmptyString(search.query),
    group: member(search.group, GROUPS),
    sort: member(search.sort, SORTS),
    direction: member(search.direction, DIRECTIONS),
    exclude: exclude.length > 0 ? exclude.join(",") : undefined,
  };
}

export function dispatchFiltersFromSearch(
  search: Partial<Record<(typeof DISPATCH_SEARCH_KEYS)[number], string | null>>,
): DispatchFilters {
  return {
    status: member(search.status, STATUSES) ?? EMPTY_DISPATCH_FILTERS.status,
    priority:
      member(search.priority, PRIORITIES) ?? EMPTY_DISPATCH_FILTERS.priority,
    assignee: search.assignee ?? EMPTY_DISPATCH_FILTERS.assignee,
    project: search.project ?? EMPTY_DISPATCH_FILTERS.project,
    label: search.label ?? EMPTY_DISPATCH_FILTERS.label,
    query: search.query ?? EMPTY_DISPATCH_FILTERS.query,
  };
}

export function dispatchFilterSearchPatch(
  filters: DispatchFilters,
): Record<
  "status" | "priority" | "assignee" | "project" | "label" | "query",
  string | null
> {
  return {
    status: filters.status === "all" ? null : filters.status,
    priority: filters.priority === "all" ? null : filters.priority,
    assignee: filters.assignee === "all" ? null : filters.assignee,
    project: filters.project === "all" ? null : filters.project,
    label: filters.label === "all" ? null : filters.label,
    query: filters.query.trim() ? filters.query : null,
  };
}

export function dispatchSearchForNavigation(
  values: Partial<Record<(typeof DISPATCH_SEARCH_KEYS)[number], string | null>>,
  workspace: string,
): DispatchRouteSearch {
  return validateDispatchSearch({ ...values, workspace });
}
