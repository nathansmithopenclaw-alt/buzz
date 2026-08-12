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

export type DispatchImportProvenance = {
  system: string;
  external_id: string;
  source_url: string | null;
  imported_at: number;
  source_created_at?: string;
  source_updated_at?: string;
  source_completed_at?: string;
  source_cancelled_at?: string;
  archived?: boolean;
  estimate_points?: number;
  export_sha256?: string;
  importer_version?: string;
  original_author?: string;
  legacy_labels?: string[];
};

export type DispatchIssue = {
  streamId: string;
  issueId: string;
  rootEvent: string;
  rootCreatedAt: number;
  tipEvent: string;
  tipCreatedAt: number;
  revision: number;
  enteredStatusAt: number;
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

export type DispatchFieldChange = {
  field: string;
  before: unknown;
  after: unknown;
};

export type DispatchActivity = {
  issueId: string;
  eventId: string;
  authorPubkey: string;
  createdAt: number;
  revision: number;
  reason: string | null;
  requestId: string | null;
  import: DispatchImportProvenance | null;
  changes: DispatchFieldChange[];
};

export type DispatchComment = {
  issueId: string;
  eventId: string;
  authorPubkey: string;
  createdAt: number;
  rootEvent: string;
  parentEvent: string | null;
  body: string;
  requestId: string | null;
  import: DispatchImportProvenance | null;
};

export type DispatchLinkedTask = {
  issueId: string;
  taskId: string;
  title: string;
  status: "assigned" | "picked_up" | "blocked" | "done" | "failed" | "verified";
  assignee: string;
  eventId: string;
  channelId: string;
  createdAt: number;
  dispatchRequestId: string | null;
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
  activity: DispatchActivity[];
  comments: DispatchComment[];
  linkedTasks: DispatchLinkedTask[];
  projects: DispatchProject[];
  diagnostics: DispatchDiagnostic[];
  taskDiagnostics: string[];
  viewerRole: "none" | "member" | "maintainer";
  sectionErrors: {
    comments: string | null;
    projects: string | null;
    tasks: string | null;
  };
  snapshotAt: string;
};
