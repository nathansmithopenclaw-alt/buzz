export type DispatchStatus =
  | "triage"
  | "backlog"
  | "todo"
  | "in_progress"
  | "in_review"
  | "done"
  | "cancelled";

export type DispatchPriority = "P0" | "P1" | "P2" | "P3";

export type DispatchDue = {
  value: number;
  precision: "instant" | "date";
  timezone: string;
};

export type DispatchRelation = {
  kind: "blocks" | "duplicate_of" | "parent";
  targetIssueId: string;
  targetRootEvent: string;
};

export type DispatchExternalRef = { type: string; ref: string };

export type DispatchIssue = {
  streamId: string;
  issueId: string;
  rootEvent: string;
  rootCreatedAt: number;
  tipEvent: string;
  identifier: string;
  title: string;
  body: string;
  status: DispatchStatus;
  priority: DispatchPriority | null;
  assignee: string | null;
  projectId: string | null;
  due: DispatchDue | null;
  estimateMinutes: number | null;
  homeChannelId: string | null;
  workflow: string;
  labels: string[];
  externalRefs: DispatchExternalRef[];
  relations: DispatchRelation[];
};

export type DispatchProject = {
  projectId: string;
  slug: string;
  name: string;
};

export type DispatchDiagnostic = {
  kind: string;
  message: string;
};

export type DispatchWorkspace = {
  id: string;
  name: string;
  trackerEventCount: number;
};

export type DispatchContext = {
  issues: DispatchIssue[];
  projects: DispatchProject[];
  diagnostics: DispatchDiagnostic[];
  snapshotAt: string;
};
