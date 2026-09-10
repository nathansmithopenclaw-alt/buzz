import type { DispatchContext, DispatchIssue } from "./types";
import { activeRecords } from "./recordTypes";
import { blockedIssueIds } from "./viewModel";

export const ATTENTION_VIEWS = {
  all: "All work",
  decision: "Needs my decision",
  ready: "Ready to assign",
  review: "Waiting on review",
  blocked: "Blocked",
  delegated: "Work I delegated",
  delivered: "Recently delivered",
} as const;
export type AttentionView = keyof typeof ATTENTION_VIEWS;
const terminal = (issue: DispatchIssue) =>
  ["done", "cancelled"].includes(issue.status);
const unfinished = (status: string) =>
  ["assigned", "picked_up", "blocked"].includes(status);

export function selectAttention(
  context: DispatchContext,
  view: AttentionView,
  viewer: string | undefined,
  now = Date.now() / 1000,
): DispatchIssue[] {
  if (view === "all") return context.issues;
  const blocked = blockedIssueIds(context.issues);
  const decisions = new Set(
    activeRecords(context.records, "decision")
      .filter(
        (value) =>
          value.record.payload.decider === viewer &&
          value.record.payload.selected_option === null,
      )
      .map((value) => value.record.payload.issue_id),
  );
  const activated = new Set(
    activeRecords(context.records, "activation")
      .filter((value) => value.record.payload.active)
      .map((value) => value.record.payload.issue_id),
  );
  const imported = new Set(
    context.activity
      .filter((value) => value.import)
      .map((value) => value.issueId),
  );
  return context.issues.filter((issue) => {
    const tasks = context.linkedTasks.filter(
      (task) => task.issueId === issue.issueId,
    );
    switch (view) {
      case "decision":
        return decisions.has(issue.issueId);
      case "ready":
        return (
          issue.status === "todo" &&
          !blocked.has(issue.issueId) &&
          !tasks.some((task) => unfinished(task.status)) &&
          (!imported.has(issue.issueId) || activated.has(issue.issueId))
        );
      case "blocked":
        return (
          !terminal(issue) &&
          (blocked.has(issue.issueId) ||
            tasks.some((task) => task.status === "blocked"))
        );
      case "review":
        return (
          !terminal(issue) &&
          (issue.status === "in_review" ||
            tasks.some((task) => task.status === "done"))
        );
      case "delegated":
        return (
          !terminal(issue) &&
          tasks.some(
            (task) => task.assigner === viewer && task.assignee !== viewer,
          )
        );
      case "delivered":
        return (
          issue.status === "done" && issue.enteredStatusAt >= now - 7 * 86400
        );
      default:
        return false;
    }
  });
}

/** Deterministic suggestions explain capacity and capability; they never dispatch. */
export function routingSuggestions(
  context: DispatchContext,
  issue: DispatchIssue,
) {
  const active = context.linkedTasks.filter((task) => unfinished(task.status));
  return activeRecords(context.records, "capacity")
    .map((record) => {
      const capacity = record.record.payload;
      const used = active.filter(
        (task) => task.assignee === capacity.executor,
      ).length;
      const matches = capacity.capabilities.filter((label) =>
        issue.labels.includes(label),
      );
      const reasons = [
        ...(capacity.paused ? ["Executor is paused"] : []),
        ...(used >= capacity.max_active ? ["At capacity"] : []),
        ...(!matches.length ? ["No matching capability label"] : []),
      ];
      return {
        executor: capacity.executor,
        used,
        limit: capacity.max_active,
        matches,
        eligible: reasons.length === 0,
        explanation: reasons.length
          ? reasons.join(" · ")
          : `Matches ${matches.join(", ")} · ${capacity.max_active - used} available slot(s)`,
      };
    })
    .sort(
      (a, b) =>
        Number(b.eligible) - Number(a.eligible) ||
        a.used - b.used ||
        a.executor.localeCompare(b.executor),
    );
}

export function assignmentWarnings(
  context: DispatchContext,
  issue: DispatchIssue,
  executor: string | null,
) {
  const chosen = routingSuggestions(context, issue).find(
    (value) => value.executor === executor,
  );
  const imported = context.activity.some(
    (value) => value.issueId === issue.issueId && value.import,
  );
  const activated = activeRecords(context.records, "activation").some(
    (value) =>
      value.record.payload.issue_id === issue.issueId &&
      value.record.payload.active,
  );
  return [
    ...(issue.status !== "todo"
      ? ["This issue has not been accepted into Todo"]
      : []),
    ...(blockedIssueIds(context.issues).has(issue.issueId)
      ? ["An unfinished dependency blocks this work"]
      : []),
    ...(activeRecords(context.records, "decision").some(
      (value) =>
        value.record.payload.issue_id === issue.issueId &&
        !value.record.payload.selected_option,
    )
      ? ["A decision is awaiting its named decider"]
      : []),
    ...(imported && !activated ? ["Imported work has not been activated"] : []),
    ...(executor && !chosen
      ? ["No configured capacity for this executor"]
      : chosen && !chosen.eligible
        ? [chosen.explanation]
        : []),
  ];
}
