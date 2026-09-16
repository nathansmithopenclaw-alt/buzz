import { DispatchAliasContext } from "./DispatchAliases";
import * as React from "react";
import { Button } from "@/shared/ui/button";
import type { DispatchIssue, DispatchProject, DispatchStatus } from "../types";
import {
  blockedIssueIds,
  filterDispatchIssues,
  groupDispatchIssues,
  sortDispatchBoardIssues,
  statusLabel,
  STATUS_ORDER,
  type DispatchFilters,
  type DispatchGroupBy,
  type DispatchProfileMap,
} from "../viewModel";
import { issueAlias } from "../issueEditing";
import { DispatchFiltersBar, IssueCard } from "./DispatchPanels";

export function DispatchBoardPanel({
  filters,
  groupBy,
  issues,
  onFiltersChange,
  onGroupByChange,
  projects,
  profiles,
  onOpenIssue,
  onChangeStatus,
  canChangeStatus,
}: {
  filters: DispatchFilters;
  groupBy: DispatchGroupBy;
  issues: DispatchIssue[];
  onFiltersChange: (filters: DispatchFilters) => void;
  onGroupByChange: (value: DispatchGroupBy) => void;
  projects: DispatchProject[];
  profiles: DispatchProfileMap;
  onOpenIssue: (id: string) => void;
  onChangeStatus: (
    issue: DispatchIssue,
    status: DispatchStatus,
  ) => Promise<void>;
  canChangeStatus: (issue: DispatchIssue, status: DispatchStatus) => boolean;
}) {
  const aliasScope = React.useContext(DispatchAliasContext);
  const alias = (issue: DispatchIssue) =>
    issueAlias(issue, aliasScope.length ? aliasScope : issues);
  const [showCompleted, setShowCompleted] = React.useState(false);
  const [limits, setLimits] = React.useState<Record<string, number>>({});
  const [error, setError] = React.useState<string | null>(null);
  const [pending, setPending] = React.useState<string | null>(null);
  const [dragged, setDragged] = React.useState<string | null>(null);
  const allFiltered = filterDispatchIssues(issues, filters);
  const terminal = (issue: DispatchIssue) =>
    issue.status === "done" || issue.status === "cancelled";
  const explicitlyTerminal =
    filters.status === "done" || filters.status === "cancelled";
  const filtered = allFiltered.filter(
    (issue) => showCompleted || explicitlyTerminal || !terminal(issue),
  );
  const grouped = groupDispatchIssues(
    sortDispatchBoardIssues(filtered, projects, profiles),
    groupBy,
    projects,
    profiles,
  );
  const groups =
    groupBy === "status"
      ? STATUS_ORDER.filter(
          (status) =>
            showCompleted ||
            explicitlyTerminal ||
            !["done", "cancelled"].includes(status),
        ).map(
          (status) =>
            grouped.find((group) => group.id === status) ?? {
              id: status,
              label: statusLabel(status),
              issues: [],
            },
        )
      : grouped;
  const blocked = blockedIssueIds(issues);
  const move = async (issue: DispatchIssue, status: DispatchStatus) => {
    if (pending || !canChangeStatus(issue, status) || status === issue.status)
      return;
    setPending(issue.issueId);
    setError(null);
    try {
      await onChangeStatus(issue, status);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setPending(null);
      setDragged(null);
    }
  };
  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      data-testid="dispatch-board-mount"
    >
      <DispatchFiltersBar
        filters={filters}
        groupBy={groupBy}
        issues={issues}
        onChange={onFiltersChange}
        onGroupByChange={onGroupByChange}
        projects={projects}
      />
      <div className="flex flex-wrap items-center gap-3 border-b border-border/40 px-5 py-2 text-xs text-muted-foreground">
        <span>
          {filtered.length} visible · {allFiltered.filter(terminal).length}{" "}
          completed or cancelled
        </span>
        <Button
          size="sm"
          variant="ghost"
          className="ml-auto"
          aria-pressed={showCompleted}
          onClick={() => setShowCompleted(!showCompleted)}
        >
          {showCompleted ? "Hide completed" : "Show completed"}
        </Button>
      </div>
      {error ? (
        <p
          className="mx-4 mt-2 rounded border border-destructive/30 p-3 text-sm text-destructive"
          role="alert"
        >
          {error}
        </p>
      ) : null}
      {!allFiltered.length ? (
        <p className="px-5 py-3 text-sm text-muted-foreground" role="status">
          No issues match these filters.
        </p>
      ) : null}
      <p className="sr-only" id="dispatch-board-help">
        Open an issue to view details. Use its Move to menu or drag a card to a
        status column to change status.
      </p>
      <div
        className="flex min-h-0 flex-1 items-stretch gap-3 overflow-x-auto p-4"
        data-testid="dispatch-board"
      >
        {groups.map((group) => (
          <section
            key={group.id}
            aria-label={`${group.label} issues`}
            data-testid={`dispatch-group-${group.id}`}
            className="flex min-h-0 w-72 shrink-0 flex-col rounded-xl border border-border/40 bg-muted/15"
            onDragOver={(event) => {
              const issue = issues.find((value) => value.issueId === dragged);
              if (
                groupBy === "status" &&
                issue &&
                !pending &&
                canChangeStatus(issue, group.id as DispatchStatus)
              )
                event.preventDefault();
            }}
            onDrop={(event) => {
              event.preventDefault();
              const issue = issues.find(
                (value) =>
                  value.issueId === dragged &&
                  value.issueId ===
                    event.dataTransfer.getData("application/buzz-issue"),
              );
              if (groupBy === "status" && issue)
                void move(issue, group.id as DispatchStatus);
            }}
          >
            <header className="flex items-center justify-between px-3 py-3">
              <h2 className="text-sm font-semibold">{group.label}</h2>
              <span className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                {group.issues.length}
              </span>
            </header>
            <div className="min-h-0 flex-1 space-y-2 overflow-y-auto px-2 pb-3">
              {group.issues.slice(0, limits[group.id] ?? 20).map((issue) => (
                <div
                  key={issue.issueId}
                  className="rounded-lg border border-border/40 bg-card"
                >
                  <IssueCard
                    draggable={!pending}
                    onDragStart={(event) => {
                      event.dataTransfer.setData(
                        "application/buzz-issue",
                        issue.issueId,
                      );
                      event.dataTransfer.effectAllowed = "move";
                      setDragged(issue.issueId);
                    }}
                    onDragEnd={() => setDragged(null)}
                    blocked={blocked.has(issue.issueId)}
                    issue={issue}
                    identifier={alias(issue)}
                    onOpenIssue={onOpenIssue}
                    profiles={profiles}
                    project={projects.find(
                      (project) => project.projectId === issue.projectId,
                    )}
                  />
                  <div className="flex items-center gap-2 px-2 pb-2">
                    <label
                      className="sr-only"
                      htmlFor={`move-${issue.issueId}`}
                    >
                      Move {alias(issue)} to
                    </label>
                    <select
                      id={`move-${issue.issueId}`}
                      aria-describedby="dispatch-board-help"
                      disabled={Boolean(pending)}
                      className="h-7 min-w-0 flex-1 rounded border border-input/50 bg-background px-1.5 text-xs text-muted-foreground"
                      value={issue.status}
                      onChange={(event) =>
                        void move(issue, event.target.value as DispatchStatus)
                      }
                    >
                      {STATUS_ORDER.map((status) => (
                        <option
                          key={status}
                          value={status}
                          disabled={!canChangeStatus(issue, status)}
                        >
                          {statusLabel(status)}
                        </option>
                      ))}
                    </select>
                    {pending === issue.issueId ? (
                      <span
                        className="text-xs text-muted-foreground"
                        role="status"
                      >
                        Saving…
                      </span>
                    ) : null}
                  </div>
                </div>
              ))}
              {group.issues.length > (limits[group.id] ?? 20) ? (
                <Button
                  className="w-full"
                  variant="ghost"
                  size="sm"
                  onClick={() =>
                    setLimits((current) => ({
                      ...current,
                      [group.id]: (current[group.id] ?? 20) + 20,
                    }))
                  }
                >
                  Show{" "}
                  {Math.min(20, group.issues.length - (limits[group.id] ?? 20))}{" "}
                  more
                </Button>
              ) : null}
              {!group.issues.length ? (
                <p className="rounded-lg border border-dashed border-border p-5 text-center text-sm text-muted-foreground">
                  No {group.label.toLowerCase()} issues
                </p>
              ) : null}
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}
