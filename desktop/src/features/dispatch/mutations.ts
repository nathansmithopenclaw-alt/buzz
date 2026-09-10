import {
  retainIssueWrite,
  acknowledgeIssueWrite,
  publishRetainedIssueWrite,
} from "./issueWriteRecovery";
import { dispatchHistoryClient } from "./relayHistory";
import type { RelayEvent } from "@/shared/api/types";
import { getRelayWsUrl, invokeTauri, signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { editRevisionFields, type DispatchIssueEdit } from "./issueEditing";
import type {
  DispatchComment,
  DispatchContext,
  DispatchIssue,
  DispatchPriority,
  DispatchStatus,
  DispatchDue,
  DispatchExternalRef,
} from "./types";

type TrackerUnsignedEvent = {
  kind: number;
  content: string;
  createdAt: number;
  tags: string[][];
};

type TrackerIssueWire = {
  v: 1;
  op: "create";
  issue_id: string;
  identifier: string;
  title: string;
  body: string;
  status: DispatchStatus;
  priority: DispatchPriority | null;
  assignee: string | null;
  project_id: string | null;
  due: DispatchDue | null;
  estimate_minutes: number | null;
  home_channel_id: string;
  source_event_id: string | null;
  workflow: "default-v1";
  labels: string[];
  external_refs: DispatchExternalRef[];
  request_id: string;
  import: null;
};

export type DispatchIssueCreateInput = {
  streamId: string;
  requestId: string;
  identifier: string;
  title: string;
  body: string;
  status: DispatchStatus;
  priority: DispatchPriority | null;
  projectId: string | null;
  labels: string[];
  assignee: string | null;
  due?: DispatchDue | null;
  estimateMinutes?: number | null;
  homeChannelId?: string;
  sourceEventId?: string | null;
  externalRefs?: DispatchExternalRef[];
};

type TrackerRevisionWire = {
  v: 1;
  op: "revise";
  issue_id: string;
  root_event: string;
  parent_event: string;
  revision: number;
  set: Record<string, unknown>;
  clear: string[];
  relations: ReturnType<typeof editRevisionFields>["relations"];
  external_refs: { add: DispatchExternalRef[]; remove: DispatchExternalRef[] };
  reason: string | null;
  request_id: string;
  import: null;
};

type TrackerCommentWire = {
  v: 1;
  issue_id: string;
  body: string;
  request_id: string;
  import: null;
};

export type DispatchIssueMutation =
  | { type: "status"; status: DispatchStatus; reason: string | null }
  | { type: "assignee"; assignee: string | null }
  | { type: "priority"; priority: DispatchPriority | null }
  | {
      type: "edit";
      fields: DispatchIssueEdit;
      expectedTip: string;
      reason?: string;
    };

type MutationDependencies = {
  identity: () => Promise<{ pubkey: string }>;
  relay?: () => Promise<string>;
  retain?: (event: RelayEvent, relay: string) => Promise<RelayEvent>;
  acknowledge?: (event: RelayEvent, relay?: string) => Promise<void>;
  buildIssue: (input: {
    streamId: string;
    root: TrackerIssueWire;
    authorPubkey: string;
  }) => Promise<TrackerUnsignedEvent>;
  buildRevision: (input: {
    streamId: string;
    revision: TrackerRevisionWire;
    authorPubkey: string;
  }) => Promise<TrackerUnsignedEvent>;
  buildComment: (input: {
    streamId: string;
    rootEvent: string;
    parentEvent: string | null;
    comment: TrackerCommentWire;
    mentions: string[];
    authorPubkey: string;
  }) => Promise<TrackerUnsignedEvent>;
  sign: (shape: TrackerUnsignedEvent) => Promise<RelayEvent>;
  publish: (
    event: RelayEvent,
    timeoutMessage: string,
    failureMessage: string,
    relay?: string,
  ) => Promise<unknown>;
  fetchRequestEvents: (streamId: string) => Promise<RelayEvent[]>;
};

// Request-id matching for issue-create idempotency. Includes 40025 so a
// request-id used by project activity cannot be reused as an issue root.
export const TRACKER_REQUEST_MATCH_KINDS = [40020, 40021, 40022, 40025, 30621];

async function fetchRequestEvents(streamId: string): Promise<RelayEvent[]> {
  return dispatchHistoryClient().fetchExhaustively({
    kinds: TRACKER_REQUEST_MATCH_KINDS,
    "#h": [streamId],
  });
}

const defaultDependencies: MutationDependencies = {
  identity: getIdentity,
  relay: getRelayWsUrl,
  retain: retainIssueWrite,
  acknowledge: acknowledgeIssueWrite,
  buildIssue: (input) =>
    invokeTauri<TrackerUnsignedEvent>("build_tracker_issue_event", input),
  buildRevision: (input) =>
    invokeTauri<TrackerUnsignedEvent>("build_tracker_revision_event", input),
  buildComment: (input) =>
    invokeTauri<TrackerUnsignedEvent>("build_tracker_comment_event", input),
  sign: (shape) =>
    signRelayEvent({
      kind: shape.kind,
      content: shape.content,
      createdAt: shape.createdAt,
      tags: shape.tags,
      allowSelfTagging: true,
    }),
  publish: (event, timeoutMessage, failureMessage, relay) =>
    publishRetainedIssueWrite(event, timeoutMessage, failureMessage, relay),
  fetchRequestEvents,
};

function hasRequestTag(event: RelayEvent, requestId: string): boolean {
  return event.tags.some((tag) => tag[0] === "request" && tag[1] === requestId);
}

function semanticTags(tags: string[][]): string[] {
  return tags
    .filter((tag) => tag[0] !== "auth")
    .map((tag) => JSON.stringify(tag))
    .sort();
}

function sameCreateRequest(
  existing: RelayEvent,
  candidate: RelayEvent,
): boolean {
  return (
    existing.kind === candidate.kind &&
    existing.pubkey === candidate.pubkey &&
    existing.content === candidate.content &&
    JSON.stringify(semanticTags(existing.tags)) ===
      JSON.stringify(semanticTags(candidate.tags))
  );
}

function normalizeLabels(values: string[]): string[] {
  const labels = new Set<string>();
  for (const value of values) {
    const normalized = value.trim().toLowerCase();
    if (!normalized) continue;
    if (new TextEncoder().encode(normalized).length > 64) {
      throw new Error("Labels must contain at most 64 UTF-8 bytes.");
    }
    labels.add(normalized);
  }
  if (labels.size > 16) {
    throw new Error("At most 16 labels are allowed.");
  }
  return [...labels].sort();
}

export async function publishDispatchIssueCreate(
  {
    input,
    refresh,
  }: {
    input: DispatchIssueCreateInput;
    refresh: () => Promise<DispatchContext>;
  },
  dependencies: MutationDependencies = defaultDependencies,
): Promise<{ issue: DispatchIssue; replayed: boolean }> {
  const title = input.title.trim();
  if (!title) throw new Error("Issue title cannot be empty.");
  const labels = normalizeLabels(input.labels);
  const relayScope = await dependencies.relay?.();
  const identity = await dependencies.identity();
  const issueId = input.requestId;
  const root: TrackerIssueWire = {
    v: 1,
    op: "create",
    issue_id: issueId,
    identifier: input.identifier,
    title,
    body: input.body,
    status: input.status,
    priority: input.priority,
    assignee: input.assignee,
    project_id: input.projectId,
    due: input.due ?? null,
    estimate_minutes: input.estimateMinutes ?? null,
    home_channel_id: input.homeChannelId ?? input.streamId,
    source_event_id: input.sourceEventId ?? null,
    workflow: "default-v1",
    labels,
    external_refs: input.externalRefs ?? [],
    request_id: input.requestId,
    import: null,
  };
  const shape = await dependencies.buildIssue({
    streamId: input.streamId,
    root,
    authorPubkey: identity.pubkey,
  });
  const candidate = await dependencies.sign(shape);
  if (candidate.pubkey !== identity.pubkey)
    throw new Error("Identity changed while signing. Your draft was retained.");
  const matches = (
    await dependencies.fetchRequestEvents(input.streamId)
  ).filter((event) => hasRequestTag(event, input.requestId));
  if (matches.length > 1) {
    throw new Error(
      `Request ${input.requestId} already names multiple Tracker events.`,
    );
  }
  let event = candidate;
  let replayed = false;
  if (matches.length === 1) {
    const existing = matches[0];
    if (!sameCreateRequest(existing, candidate)) {
      throw new Error(
        `Request ${input.requestId} was already used with different Tracker input.`,
      );
    }
    event = existing;
    replayed = true;
  } else {
    event =
      dependencies.retain && relayScope
        ? await dependencies.retain(candidate, relayScope)
        : candidate;
    await dependencies.publish(
      event,
      "Timed out creating issue. Retry keeps the same request ID.",
      "Failed to create issue.",
      relayScope,
    );
  }
  const context = await refresh();
  const issue = context.issues.find(
    (value) => value.issueId === issueId && value.rootEvent === event.id,
  );
  if (!issue) {
    throw new Error(
      "Issue reached the relay but is not visible in the canonical fold yet. Retry to reconcile it.",
    );
  }
  await dependencies.acknowledge?.(event, relayScope);
  return { issue, replayed };
}

export class TrackerMutationConflictError extends Error {
  constructor() {
    super(
      "The issue changed while your update was publishing. Buzz restored the canonical version.",
    );
    this.name = "TrackerMutationConflictError";
  }
}

export class TrackerRevisionRejectedError extends Error {
  constructor(reason: string) {
    super(
      `The update was not applied. The issue history rejected the revision: ${reason}`,
    );
    this.name = "TrackerRevisionRejectedError";
  }
}

export function foldRejectionForEvent(
  diagnostics: DispatchContext["diagnostics"],
  eventId: string,
): string | null {
  return (
    diagnostics.find((diagnostic) => diagnostic.message.includes(eventId))
      ?.message ?? null
  );
}

export function canApplyDispatchMutation(
  issue: DispatchIssue,
  mutation: DispatchIssueMutation,
  viewerRole: DispatchContext["viewerRole"],
): boolean {
  if (viewerRole === "none") return false;
  if (
    mutation.type === "edit" &&
    mutation.fields.identifier !== undefined &&
    mutation.fields.identifier !== issue.identifier &&
    viewerRole !== "maintainer"
  )
    return false;
  const nextStatus =
    mutation.type === "status"
      ? mutation.status
      : mutation.type === "edit"
        ? mutation.fields.status
        : undefined;
  if (nextStatus === undefined) return true;
  const terminal = (status: DispatchStatus) =>
    status === "done" || status === "cancelled";
  return !terminal(issue.status) && !terminal(nextStatus)
    ? true
    : viewerRole === "maintainer";
}

function mutationFields(
  mutation: DispatchIssueMutation,
  issue: DispatchIssue,
): {
  set: Record<string, unknown>;
  clear: string[];
  reason: string | null;
} {
  switch (mutation.type) {
    case "edit": {
      if (mutation.expectedTip !== issue.tipEvent)
        throw new TrackerMutationConflictError();
      const fields = editRevisionFields(issue, mutation.fields);
      return {
        set: fields.set,
        clear: fields.clear,
        reason: mutation.reason?.trim() || null,
      };
    }
    case "status": {
      if (mutation.reason !== null && !mutation.reason.trim()) {
        throw new Error("Status change reason must be non-empty when present.");
      }
      return {
        set: { status: mutation.status },
        clear: [],
        reason: mutation.reason?.trim() ?? null,
      };
    }
    case "assignee":
      return mutation.assignee
        ? { set: { assignee: mutation.assignee }, clear: [], reason: null }
        : { set: {}, clear: ["assignee"], reason: null };
    case "priority":
      return mutation.priority
        ? { set: { priority: mutation.priority }, clear: [], reason: null }
        : { set: {}, clear: ["priority"], reason: null };
  }
}

function optimisticIssue(
  issue: DispatchIssue,
  mutation: DispatchIssueMutation,
  event: RelayEvent,
): DispatchIssue {
  const base = {
    ...issue,
    tipEvent: event.id,
    tipCreatedAt: event.created_at,
    revision: issue.revision + 1,
  };
  switch (mutation.type) {
    case "edit":
      return { ...base, ...mutation.fields };
    case "status":
      return {
        ...base,
        status: mutation.status,
        enteredStatusAt:
          mutation.status === issue.status
            ? issue.enteredStatusAt
            : event.created_at,
      };
    case "assignee":
      return { ...base, assignee: mutation.assignee };
    case "priority":
      return { ...base, priority: mutation.priority };
  }
}

export async function publishDispatchIssueMutation(
  {
    issue,
    mutation,
    viewerRole,
    onOptimistic,
    onRollback,
    refresh,
  }: {
    issue: DispatchIssue;
    mutation: DispatchIssueMutation;
    viewerRole: DispatchContext["viewerRole"];
    onOptimistic: (issue: DispatchIssue) => void;
    onRollback: (issue: DispatchIssue) => void;
    refresh: () => Promise<DispatchContext>;
  },
  dependencies: MutationDependencies = defaultDependencies,
): Promise<DispatchIssue> {
  if (!canApplyDispatchMutation(issue, mutation, viewerRole)) {
    throw new Error("Your Tracker role does not allow this issue transition.");
  }
  const relayScope = await dependencies.relay?.();
  const identity = await dependencies.identity();
  const fields = mutationFields(mutation, issue);
  const edits =
    mutation.type === "edit"
      ? editRevisionFields(issue, mutation.fields)
      : null;
  const revision: TrackerRevisionWire = {
    v: 1,
    op: "revise",
    issue_id: issue.issueId,
    root_event: issue.rootEvent,
    parent_event: issue.tipEvent,
    revision: issue.revision + 1,
    set: fields.set,
    clear: fields.clear,
    relations: edits?.relations ?? { add: [], remove: [] },
    external_refs: edits?.external_refs ?? { add: [], remove: [] },
    reason: fields.reason,
    request_id: crypto.randomUUID(),
    import: null,
  };
  const shape = await dependencies.buildRevision({
    streamId: issue.streamId,
    revision,
    authorPubkey: identity.pubkey,
  });
  const signed = await dependencies.sign(shape);
  if (signed.pubkey !== identity.pubkey)
    throw new Error("Identity changed while signing. Your draft was retained.");
  const event =
    dependencies.retain && relayScope
      ? await dependencies.retain(signed, relayScope)
      : signed;
  const optimistic = optimisticIssue(issue, mutation, event);
  onOptimistic(optimistic);
  try {
    await dependencies.publish(
      event,
      "Timed out updating issue.",
      "Failed to update issue.",
      relayScope,
    );
    const context = await refresh();
    const canonical = context.issues.find(
      (candidate) => candidate.issueId === issue.issueId,
    );
    const rejection = foldRejectionForEvent(context.diagnostics, event.id);
    if (rejection) {
      onRollback(issue);
      throw new TrackerRevisionRejectedError(rejection);
    }
    const applied = context.activity.some(
      (entry) => entry.issueId === issue.issueId && entry.eventId === event.id,
    );
    if (!canonical || (canonical.tipEvent !== event.id && !applied)) {
      onRollback(issue);
      await refresh();
      throw new TrackerMutationConflictError();
    }
    await dependencies.acknowledge?.(event, relayScope);
    return canonical;
  } catch (error) {
    if (
      !(error instanceof TrackerMutationConflictError) &&
      !(error instanceof TrackerRevisionRejectedError)
    ) {
      onRollback(issue);
    }
    throw error;
  }
}

export async function publishDispatchComment(
  {
    body,
    issue,
    mentions = [],
    onOptimistic,
    onRollback,
    refresh,
  }: {
    body: string;
    issue: DispatchIssue;
    mentions?: string[];
    onOptimistic: (comment: DispatchComment) => void;
    onRollback: (eventId: string) => void;
    refresh: () => Promise<DispatchContext>;
  },
  dependencies: MutationDependencies = defaultDependencies,
): Promise<string> {
  const trimmedBody = body.trim();
  if (!trimmedBody) throw new Error("Comment cannot be empty.");
  const relayScope = await dependencies.relay?.();
  const identity = await dependencies.identity();
  const comment: TrackerCommentWire = {
    v: 1,
    issue_id: issue.issueId,
    body: trimmedBody,
    request_id: crypto.randomUUID(),
    import: null,
  };
  const shape = await dependencies.buildComment({
    streamId: issue.streamId,
    rootEvent: issue.rootEvent,
    parentEvent: null,
    comment,
    mentions,
    authorPubkey: identity.pubkey,
  });
  const signed = await dependencies.sign(shape);
  if (signed.pubkey !== identity.pubkey)
    throw new Error("Identity changed while signing. Your draft was retained.");
  const event =
    dependencies.retain && relayScope
      ? await dependencies.retain(signed, relayScope)
      : signed;
  onOptimistic({
    issueId: issue.issueId,
    eventId: event.id,
    authorPubkey: identity.pubkey,
    createdAt: event.created_at,
    rootEvent: issue.rootEvent,
    parentEvent: null,
    body: comment.body,
    requestId: comment.request_id,
    import: null,
  });
  try {
    await dependencies.publish(
      event,
      "Timed out adding comment.",
      "Failed to add comment.",
      relayScope,
    );
    const context = await refresh();
    if (
      !context.comments.some(
        (comment) =>
          comment.eventId === event.id && comment.issueId === issue.issueId,
      )
    )
      throw new Error(
        "The comment was sent but is not yet confirmed in accepted history. Your exact comment is saved for retry.",
      );
    await dependencies.acknowledge?.(event, relayScope);
    return event.id;
  } catch (error) {
    onRollback(event.id);
    throw error;
  }
}
