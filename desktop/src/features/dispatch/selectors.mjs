import { buildMessageLink } from "../messages/lib/messageLink.ts";

const PRIORITY_ORDER = new Map([
  ["P0", 0],
  ["P1", 1],
  ["P2", 2],
  ["P3", 3],
]);

const TERMINAL_STATUSES = new Set(["done", "cancelled"]);

export function priorityRank(priority) {
  return PRIORITY_ORDER.get(priority) ?? Number.MAX_SAFE_INTEGER;
}

export function compareTriageIssues(left, right) {
  return (
    priorityRank(left.priority) - priorityRank(right.priority) ||
    left.enteredStatusAt - right.enteredStatusAt ||
    left.identifier.localeCompare(right.identifier)
  );
}

export function splitTriageIssues(issues) {
  const triage = issues.filter((issue) => issue.status === "triage");
  return {
    unassigned: triage
      .filter((issue) => !issue.assignee)
      .sort(compareTriageIssues),
    assigned: triage
      .filter((issue) => issue.assignee)
      .sort(compareTriageIssues),
  };
}

export function compareMyIssues(left, right) {
  const leftDue = left.due?.value ?? Number.MAX_SAFE_INTEGER;
  const rightDue = right.due?.value ?? Number.MAX_SAFE_INTEGER;
  return (
    priorityRank(left.priority) - priorityRank(right.priority) ||
    leftDue - rightDue ||
    left.identifier.localeCompare(right.identifier)
  );
}

export function selectMyIssues(issues, selfPubkey) {
  const normalizedSelf = selfPubkey?.toLowerCase();
  if (!normalizedSelf) return [];
  return issues
    .filter(
      (issue) =>
        issue.assignee?.toLowerCase() === normalizedSelf &&
        !TERMINAL_STATUSES.has(issue.status),
    )
    .sort(compareMyIssues);
}

export function chronologicalActivity(activity, comments) {
  return [
    ...activity.map((entry) => ({ ...entry, timelineKind: "activity" })),
    ...comments.map((entry) => ({ ...entry, timelineKind: "comment" })),
  ].sort(
    (left, right) =>
      left.createdAt - right.createdAt ||
      left.eventId.localeCompare(right.eventId),
  );
}

export function taskMessageLink(task) {
  return buildMessageLink({
    channelId: task.channelId,
    messageId: task.eventId,
  });
}
