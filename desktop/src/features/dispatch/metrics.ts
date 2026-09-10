import type { DispatchContext, DispatchStatus } from "./types";

const terminal = (value: unknown) => value === "done" || value === "cancelled";
export function dispatchMetrics(
  context: DispatchContext,
  now = Date.now() / 1000,
) {
  const visibleIds = new Set(context.issues.map((issue) => issue.issueId));
  const imported = new Set(
    context.activity
      .filter((entry) => entry.import && visibleIds.has(entry.issueId))
      .map((entry) => entry.issueId),
  );
  const queueAges: Record<string, number[]> = { triage: [], in_review: [] };
  const completedDurations: Record<string, number[]> = {
    triage: [],
    in_review: [],
  };
  let reopens = 0,
    missingStatusHistory = 0;
  for (const issue of context.issues) {
    const activity = context.activity
      .filter((entry) => entry.issueId === issue.issueId && !entry.import)
      .sort((a, b) => a.revision - b.revision);
    let start: number | null = imported.has(issue.issueId)
      ? null
      : issue.rootCreatedAt;
    let status: DispatchStatus | null = null;
    for (const entry of activity) {
      for (const change of entry.changes.filter(
        (change) => change.field === "status",
      )) {
        if (terminal(change.before) && !terminal(change.after)) reopens++;
        if (
          start !== null &&
          status !== null &&
          status === change.before &&
          completedDurations[status] &&
          entry.createdAt >= start
        )
          completedDurations[status].push(entry.createdAt - start);
        else if (
          start !== null &&
          status === null &&
          completedDurations[String(change.before)] &&
          entry.createdAt >= start
        )
          completedDurations[String(change.before)].push(
            entry.createdAt - start,
          );
        start = entry.createdAt;
        status = change.after as DispatchStatus;
      }
    }
    if (queueAges[issue.status]) {
      const observedEntry =
        !imported.has(issue.issueId) ||
        activity.some((entry) =>
          entry.changes.some(
            (change) =>
              change.field === "status" && change.after === issue.status,
          ),
        );
      if (observedEntry && issue.enteredStatusAt <= now)
        queueAges[issue.status].push(now - issue.enteredStatusAt);
      else missingStatusHistory++;
    }
  }
  const blockedDurations: number[] = [];
  let blockedMissing = 0;
  for (const task of context.linkedTasks) {
    const history = task.history ?? [];
    for (let index = 0; index < history.length; index++) {
      const entry = history[index];
      if (
        entry.status !== "blocked" ||
        history[index - 1]?.status === "blocked"
      )
        continue;
      const next = history
        .slice(index + 1)
        .find((value) => value.status !== "blocked");
      const end = next?.createdAt ?? (task.status === "blocked" ? now : null);
      if (end !== null && end >= entry.createdAt)
        blockedDurations.push(end - entry.createdAt);
    }
    if (
      task.status === "blocked" &&
      !history.some((entry) => entry.status === "blocked")
    )
      blockedMissing++;
  }
  const attempts = Object.fromEntries(
    ["assigned", "picked_up", "blocked", "done", "verified", "failed"].map(
      (status) => [
        status,
        context.linkedTasks.filter((task) => task.status === status).length,
      ],
    ),
  );
  return {
    nativeIssues: context.issues.length - imported.size,
    importedIssues: imported.size,
    queueAges,
    completedDurations,
    reopens,
    missingStatusHistory,
    blockedDurations,
    blockedMissing,
    attempts,
  };
}
export function medianDuration(values: number[]) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b),
    midpoint = Math.floor(sorted.length / 2);
  return sorted.length % 2
    ? sorted[midpoint]
    : (sorted[midpoint - 1] + sorted[midpoint]) / 2;
}
export function durationLabel(seconds: number | null) {
  if (seconds === null) return "Not observed";
  if (seconds < 3600) return `${Math.round(seconds / 60)} min`;
  if (seconds < 86400) return `${(seconds / 3600).toFixed(1)} hr`;
  return `${(seconds / 86400).toFixed(1)} days`;
}
