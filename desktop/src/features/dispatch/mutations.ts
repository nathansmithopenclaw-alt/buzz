import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { invokeTauri, signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import type {
  DispatchComment,
  DispatchContext,
  DispatchIssue,
  DispatchPriority,
  DispatchStatus,
} from "./types";

type TrackerUnsignedEvent = {
  kind: number;
  content: string;
  createdAt: number;
  tags: string[][];
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
  relations: { add: never[]; remove: never[] };
  external_refs: { add: never[]; remove: never[] };
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
  | { type: "priority"; priority: DispatchPriority | null };

type MutationDependencies = {
  identity: () => Promise<{ pubkey: string }>;
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
  ) => Promise<unknown>;
};

const defaultDependencies: MutationDependencies = {
  identity: getIdentity,
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
  publish: (event, timeoutMessage, failureMessage) =>
    relayClient.publishEvent(event, timeoutMessage, failureMessage),
};

export class TrackerMutationConflictError extends Error {
  constructor() {
    super(
      "The issue changed while your update was publishing. Buzz restored the canonical version.",
    );
    this.name = "TrackerMutationConflictError";
  }
}

export function canApplyDispatchMutation(
  issue: DispatchIssue,
  mutation: DispatchIssueMutation,
  viewerRole: DispatchContext["viewerRole"],
): boolean {
  if (viewerRole === "none") return false;
  if (mutation.type !== "status") return true;
  const terminal = (status: DispatchStatus) =>
    status === "done" || status === "cancelled";
  return !terminal(issue.status) && !terminal(mutation.status)
    ? true
    : viewerRole === "maintainer";
}

function mutationFields(mutation: DispatchIssueMutation): {
  set: Record<string, unknown>;
  clear: string[];
  reason: string | null;
} {
  switch (mutation.type) {
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
  const identity = await dependencies.identity();
  const fields = mutationFields(mutation);
  const revision: TrackerRevisionWire = {
    v: 1,
    op: "revise",
    issue_id: issue.issueId,
    root_event: issue.rootEvent,
    parent_event: issue.tipEvent,
    revision: issue.revision + 1,
    set: fields.set,
    clear: fields.clear,
    relations: { add: [], remove: [] },
    external_refs: { add: [], remove: [] },
    reason: fields.reason,
    request_id: crypto.randomUUID(),
    import: null,
  };
  const shape = await dependencies.buildRevision({
    streamId: issue.streamId,
    revision,
    authorPubkey: identity.pubkey,
  });
  const event = await dependencies.sign(shape);
  const optimistic = optimisticIssue(issue, mutation, event);
  onOptimistic(optimistic);
  try {
    await dependencies.publish(
      event,
      "Timed out updating issue.",
      "Failed to update issue.",
    );
    const context = await refresh();
    const canonical = context.issues.find(
      (candidate) => candidate.issueId === issue.issueId,
    );
    const applied = context.activity.some(
      (entry) => entry.issueId === issue.issueId && entry.eventId === event.id,
    );
    if (!canonical || (canonical.tipEvent !== event.id && !applied)) {
      onRollback(issue);
      await refresh();
      throw new TrackerMutationConflictError();
    }
    return canonical;
  } catch (error) {
    if (!(error instanceof TrackerMutationConflictError)) onRollback(issue);
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
  const event = await dependencies.sign(shape);
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
    );
    await refresh();
    return event.id;
  } catch (error) {
    onRollback(event.id);
    throw error;
  }
}
