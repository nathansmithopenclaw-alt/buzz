import type {
  DispatchActivity,
  DispatchComment,
  DispatchIssue,
  DispatchLinkedTask,
} from "./types";

export function priorityRank(priority: DispatchIssue["priority"]): number;
export function compareTriageIssues(
  left: DispatchIssue,
  right: DispatchIssue,
): number;
export function splitTriageIssues(issues: readonly DispatchIssue[]): {
  unassigned: DispatchIssue[];
  assigned: DispatchIssue[];
};
export function compareMyIssues(
  left: DispatchIssue,
  right: DispatchIssue,
): number;
export function selectMyIssues(
  issues: readonly DispatchIssue[],
  selfPubkey: string | null | undefined,
): DispatchIssue[];
export function chronologicalActivity(
  activity: readonly DispatchActivity[],
  comments: readonly DispatchComment[],
): Array<
  | (DispatchActivity & { timelineKind: "activity" })
  | (DispatchComment & { timelineKind: "comment" })
>;
export function taskMessageLink(task: DispatchLinkedTask): string;
