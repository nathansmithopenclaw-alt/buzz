import type { DispatchDiagnostic, DispatchIssue } from "./types";
import type { DispatchIssueEdit } from "./issueEditing";

export function unresolvedDiagnostics(
  diagnostics: DispatchDiagnostic[],
  issues: readonly DispatchIssue[],
) {
  return diagnostics.filter(
    (diagnostic) =>
      !diagnostic.eventId ||
      !issues.some(
        (issue) =>
          issue.issueId === diagnostic.issueId &&
          issue.externalRefs.some(
            (ref) =>
              ref.type === "resolved_conflict" &&
              ref.ref === diagnostic.eventId,
          ),
      ),
  );
}
const fields: Record<string, keyof DispatchIssueEdit> = {
  title: "title",
  body: "body",
  status: "status",
  priority: "priority",
  assignee: "assignee",
  project_id: "projectId",
  labels: "labels",
  due: "due",
  estimate_minutes: "estimateMinutes",
  home_channel_id: "homeChannelId",
  identifier: "identifier",
};
export function proposedConflictFields(
  diagnostic: DispatchDiagnostic,
  issue?: DispatchIssue,
): DispatchIssueEdit {
  const proposed: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(diagnostic.proposed?.set ?? {}))
    if (fields[key]) proposed[fields[key]] = value;
  for (const key of diagnostic.proposed?.clear ?? [])
    if (fields[key]) proposed[fields[key]] = null;
  if (issue) {
    const operations = diagnostic.proposed;
    const normalizeRelation = (value: unknown) => {
      if (!value || typeof value !== "object") return null;
      const ref = value as Record<string, unknown>;
      if (
        !["blocks", "duplicate_of", "parent"].includes(String(ref.type)) ||
        typeof ref.target_issue_id !== "string" ||
        typeof ref.target_root_event !== "string"
      )
        return null;
      return {
        kind: ref.type as DispatchIssue["relations"][number]["kind"],
        targetIssueId: ref.target_issue_id,
        targetRootEvent: ref.target_root_event,
      };
    };
    if (
      operations?.relations &&
      (operations.relations.add.length || operations.relations.remove.length)
    ) {
      const add = operations.relations.add
          .map(normalizeRelation)
          .filter((value) => value !== null),
        remove = operations.relations.remove
          .map(normalizeRelation)
          .filter((value) => value !== null);
      const key = (value: DispatchIssue["relations"][number]) =>
        `${value.kind}:${value.targetIssueId}:${value.targetRootEvent}`;
      const next = new Map(
        issue.relations
          .filter((value) => !remove.some((ref) => key(ref) === key(value)))
          .map((value) => [key(value), value]),
      );
      for (const value of add) next.set(key(value), value);
      proposed.relations = [...next.values()];
    }
    if (
      operations?.external_refs &&
      (operations.external_refs.add.length ||
        operations.external_refs.remove.length)
    ) {
      const key = (value: DispatchIssue["externalRefs"][number]) =>
        `${value.type}:${value.ref}`;
      const next = new Map(
        issue.externalRefs
          .filter(
            (value) =>
              !operations.external_refs?.remove.some(
                (ref) => key(ref) === key(value),
              ),
          )
          .map((value) => [key(value), value]),
      );
      for (const value of operations.external_refs.add)
        next.set(key(value), value);
      proposed.externalRefs = [...next.values()];
    }
  }
  return proposed as DispatchIssueEdit;
}
export const conflictTitle: Record<string, string> = {
  LosingBranch: "An edit needs reconciliation",
  RootCollision: "Two records claim the same issue",
  IdentifierCollision: "An identifier is used twice",
  InvalidRelation: "A relationship needs review",
  BlocksCycle: "A dependency loop blocks this work",
  InvalidEvent: "An update was not accepted",
  PlanningConflict: "A shared planning record needs review",
};
