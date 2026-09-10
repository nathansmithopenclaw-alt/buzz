import type { DispatchContext, DispatchIssue } from "./types";
import { activeRecords } from "./recordTypes";

/** A bounded snapshot of already-authorized records, never an implicit source-channel read. */
export function dispatchContextExport(
  context: DispatchContext,
  issue: DispatchIssue,
  since = 0,
  budget = 16000,
): string {
  const limit = Math.max(
    1024,
    Math.min(32000, Number.isFinite(budget) ? budget : 16000),
  );
  const size = (value: string) => new TextEncoder().encode(value).length;
  const truncate = (value: string, bytes: number) => {
    let result = "",
      used = 0;
    for (const character of value) {
      const length = size(character);
      if (used + length > bytes) break;
      result += character;
      used += length;
    }
    return result;
  };
  const attempts = context.linkedTasks
    .filter((task) => task.issueId === issue.issueId)
    .sort(
      (a, b) => a.createdAt - b.createdAt || a.eventId.localeCompare(b.eventId),
    );
  const decisions = activeRecords(context.records, "decision")
    .filter((value) => value.record.payload.issue_id === issue.issueId)
    .sort(
      (a, b) =>
        a.created_at - b.created_at || a.event_id.localeCompare(b.event_id),
    );
  const changes = context.activity
    .filter(
      (value) => value.issueId === issue.issueId && value.createdAt > since,
    )
    .sort(
      (a, b) => a.createdAt - b.createdAt || a.eventId.localeCompare(b.eventId),
    )
    .slice(-40);
  const record = {
    workspace: issue.streamId,
    issueId: issue.issueId,
    currentEvent: issue.tipEvent,
    snapshotAt: context.snapshotAt,
    scope: issue.body,
    title: issue.title,
    owner: issue.assignee,
    status: issue.status,
    priority: issue.priority,
    homeChannel: issue.homeChannelId,
    relations: issue.relations,
    sourceReferences: issue.externalRefs,
    attempts,
    decisions,
    changesSince: since,
    changes,
    readLimitations: context.sectionErrors,
  };
  const full = JSON.stringify(record, null, 2);
  if (size(full) <= limit) return full;
  // Keep the envelope valid and make truncation explicit, rather than cutting JSON mid-field.
  const bounded = {
    ...record,
    scope: truncate(issue.body, Math.floor(limit / 3)),
    attempts: attempts.slice(-3),
    changes: changes.slice(-8),
    decisions: decisions.slice(-5),
    truncated: true,
    fullContextAvailableInDispatch: true,
  };
  let text = JSON.stringify(bounded, null, 2);
  if (size(text) > limit)
    text = JSON.stringify(
      {
        workspace: issue.streamId,
        issueId: issue.issueId,
        currentEvent: issue.tipEvent,
        snapshotAt: context.snapshotAt,
        title: issue.title,
        scope: truncate(issue.body, Math.floor(limit / 2)),
        truncated: true,
        notice:
          "Additional authorized history and evidence must be read in Dispatch.",
      },
      null,
      2,
    );
  if (size(text) > limit) {
    const minimal = JSON.parse(text);
    minimal.title = truncate(String(minimal.title), 100);
    while (
      size(JSON.stringify(minimal, null, 2)) > limit &&
      minimal.scope.length
    )
      minimal.scope = truncate(
        minimal.scope,
        Math.floor(size(minimal.scope) / 2),
      );
    text = JSON.stringify(minimal, null, 2);
  }
  return text;
}

/** Title overlap is advisory evidence for review, not proof of a duplicate. */
export function relatedIssueSuggestions(
  issue: DispatchIssue,
  issues: readonly DispatchIssue[],
) {
  const words = (value: string) =>
    new Set(
      value
        .toLowerCase()
        .split(/[^\p{L}\p{N}]+/u)
        .filter((word) => word.length > 2),
    );
  const source = words(issue.title);
  return issues
    .filter((candidate) => candidate.issueId !== issue.issueId)
    .map((candidate) => {
      const target = words(candidate.title);
      const overlap = [...source].filter((word) => target.has(word));
      const score =
        overlap.length / (source.size + target.size - overlap.length || 1);
      return { issue: candidate, score, sharedWords: overlap };
    })
    .filter((value) => value.sharedWords.length >= 2 && value.score >= 0.35)
    .sort(
      (a, b) =>
        b.score - a.score || a.issue.issueId.localeCompare(b.issue.issueId),
    )
    .slice(0, 5);
}
