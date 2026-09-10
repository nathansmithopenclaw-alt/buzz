import type {
  DispatchContext,
  DispatchIssue,
  DispatchPriority,
  DispatchStatus,
} from "@/features/dispatch/types";
import type { DispatchRecord } from "@/features/dispatch/recordTypes";
import type { RelayEvent } from "@/shared/api/types";

const applied = new WeakMap<DispatchContext, Set<string>>();
/** UI fixture projection advances only after a relay publication, never during signing. */
export function applyDispatchMockEvent(
  context: DispatchContext,
  signed: RelayEvent,
  rejectRevision = false,
) {
  const ids = applied.get(context) ?? new Set<string>();
  if (ids.has(signed.id)) return;
  ids.add(signed.id);
  applied.set(context, ids);
  if (
    signed.kind === 9 &&
    signed.tags.some((tag) => tag[0] === "t" && tag[1] === "buzz-task")
  ) {
    const fence = "```buzz-task\n",
      body = signed.content
        .slice(signed.content.lastIndexOf(fence) + fence.length)
        .trim()
        .replace(/\n```$/, "");
    const envelope = JSON.parse(body);
    const issue = context.issues.find(
      (issue) => issue.issueId === envelope.tracker_issue_id,
    );
    const previous = context.linkedTasks.find(
      (task) => task.taskId === envelope.task_id,
    );
    if (
      !issue ||
      (previous
        ? envelope.authority?.previous_event_id !== previous.eventId ||
          envelope.authority?.assignment_event_id !== previous.assignmentEventId
        : !issue.externalRefs.some(
            (ref) => ref.type === "task_event" && ref.ref === signed.id,
          ))
    )
      return;
    context.linkedTasks = [
      ...context.linkedTasks.filter((task) => task.taskId !== envelope.task_id),
      {
        issueId: issue.issueId,
        taskId: envelope.task_id,
        title: envelope.title,
        status: envelope.status,
        assignee: envelope.assignee,
        eventId: signed.id,
        channelId: signed.tags.find((tag) => tag[0] === "h")?.[1] ?? "",
        createdAt: signed.created_at,
        dispatchRequestId: envelope.dispatch_request_id,
        assigner: envelope.assigner,
        reviewer: envelope.authority?.reviewer,
        assignmentEventId: previous?.assignmentEventId ?? signed.id,
        assignedAt: previous?.assignedAt ?? signed.created_at,
        notes: envelope.notes,
        blockedOn: envelope.blocked_on,
        history: [
          ...(previous?.history ?? []),
          {
            eventId: signed.id,
            authorPubkey: signed.pubkey,
            createdAt: signed.created_at,
            status: envelope.status,
            notes: envelope.notes ?? null,
            blockedOn: envelope.blocked_on ?? null,
            evidence: envelope.verification?.evidence_ref ?? null,
          },
        ],
        verification: envelope.verification,
      },
    ];
    return;
  }
  if (signed.kind === 40026) {
    const record = JSON.parse(signed.content) as DispatchRecord;
    const existing = context.records?.find(
      (value) => value.record.record_id === record.record_id,
    );
    if (existing && existing.event_id !== record.parent_event) {
      context.diagnostics.push({
        kind: "PlanningConflict",
        message: `${signed.id}: stale planning revision`,
      });
      return;
    }
    context.records = [
      ...(context.records ?? []).filter(
        (value) => value.record.record_id !== record.record_id,
      ),
      {
        creator: existing?.creator ?? signed.pubkey,
        author: signed.pubkey,
        event_id: signed.id,
        root_event: existing?.root_event ?? signed.id,
        accepted_events: [...(existing?.accepted_events ?? []), signed.id],
        created_at: signed.created_at,
        record,
      },
    ];
    return;
  }
  if (
    context &&
    (signed.kind === 40020 || signed.kind === 40021 || signed.kind === 40022)
  ) {
    const content = JSON.parse(signed.content) as Record<string, unknown>;
    const issueId = String(content.issue_id ?? "");
    const issue = context.issues.find(
      (candidate) => candidate.issueId === issueId,
    );
    if (signed.kind === 40020 && !issue) {
      context.issues.push({
        streamId:
          signed.tags.find((tag: string[]) => tag[0] === "h")?.[1] ?? "",
        issueId,
        rootEvent: signed.id,
        rootCreatedAt: signed.created_at,
        tipEvent: signed.id,
        tipCreatedAt: signed.created_at,
        revision: 0,
        enteredStatusAt: signed.created_at,
        identifier: String(content.identifier ?? ""),
        title: String(content.title ?? ""),
        body: String(content.body ?? ""),
        status: String(content.status ?? "triage") as DispatchStatus,
        priority: (content.priority ?? null) as DispatchPriority | null,
        assignee:
          typeof content.assignee === "string" ? content.assignee : null,
        projectId:
          typeof content.project_id === "string" ? content.project_id : null,
        due: (content.due ?? null) as DispatchIssue["due"],
        estimateMinutes: (content.estimate_minutes ?? null) as number | null,
        sourceEventId:
          typeof content.source_event_id === "string"
            ? content.source_event_id
            : null,
        homeChannelId:
          typeof content.home_channel_id === "string"
            ? content.home_channel_id
            : null,
        workflow: String(content.workflow ?? "default-v1"),
        labels: Array.isArray(content.labels) ? content.labels.map(String) : [],
        externalRefs: (content.external_refs ??
          []) as DispatchIssue["externalRefs"],
        relations: [],
      });
    } else if (signed.kind === 40021 && issue) {
      const set = (content.set ?? {}) as Record<string, unknown>;
      const reasonText =
        typeof content.reason === "string" ? content.reason : null;
      const rejectBlankStatusReason =
        Boolean(set.status) && reasonText !== null && !reasonText.trim();
      const rejectForced = rejectRevision;
      if (rejectBlankStatusReason || rejectForced) {
        context.diagnostics.push({
          kind: "InvalidEvent",
          message: `InvalidEvent { event_id: "${signed.id}", reason: "status change reason must be non-empty when present" }`,
        });
      } else {
        const clear = Array.isArray(content.clear)
          ? content.clear.map(String)
          : [];
        const property = (field: string) =>
          (
            ({
              project_id: "projectId",
              estimate_minutes: "estimateMinutes",
              home_channel_id: "homeChannelId",
            }) as Record<string, string>
          )[field] ?? field;
        const changes = Object.entries(set).map(([field, after]) => ({
          field,
          before: issue[property(field) as keyof typeof issue] ?? null,
          after,
        }));
        for (const [field, value] of Object.entries(set))
          (issue as unknown as Record<string, unknown>)[property(field)] =
            value;
        for (const field of clear) {
          changes.push({
            field,
            before: issue[property(field) as keyof typeof issue] ?? null,
            after: null,
          });
          (issue as unknown as Record<string, unknown>)[property(field)] = null;
        }
        const relations = content.relations as
          | {
              add?: {
                type: DispatchIssue["relations"][number]["kind"];
                target_issue_id: string;
                target_root_event: string;
              }[];
              remove?: { type: string; target_issue_id: string }[];
            }
          | undefined;
        issue.relations = issue.relations.filter(
          (relation) =>
            !relations?.remove?.some(
              (item) =>
                item.type === relation.kind &&
                item.target_issue_id === relation.targetIssueId,
            ),
        );
        for (const relation of relations?.add ?? [])
          issue.relations.push({
            kind: relation.type,
            targetIssueId: relation.target_issue_id,
            targetRootEvent: relation.target_root_event,
          });
        const refs = content.external_refs as
          | {
              add?: DispatchIssue["externalRefs"];
              remove?: DispatchIssue["externalRefs"];
            }
          | undefined;
        const previousRefs = [...issue.externalRefs];
        issue.externalRefs = issue.externalRefs.filter(
          (ref) =>
            !refs?.remove?.some(
              (item) => item.type === ref.type && item.ref === ref.ref,
            ),
        );
        issue.externalRefs.push(...(refs?.add ?? []));
        if (JSON.stringify(previousRefs) !== JSON.stringify(issue.externalRefs))
          changes.push({
            field: "externalRefs",
            before: previousRefs,
            after: [...issue.externalRefs],
          });
        if (Object.hasOwn(set, "status"))
          issue.enteredStatusAt = signed.created_at;
        issue.tipEvent = signed.id;
        issue.tipCreatedAt = signed.created_at;
        issue.revision = Number(content.revision);
        context.activity.push({
          issueId,
          eventId: signed.id,
          authorPubkey: signed.pubkey,
          createdAt: signed.created_at,
          revision: issue.revision,
          reason: reasonText,
          requestId:
            typeof content.request_id === "string" ? content.request_id : null,
          import: null,
          changes,
        });
      }
    } else if (signed.kind === 40022 && issue) {
      context.comments.push({
        issueId,
        eventId: signed.id,
        authorPubkey: signed.pubkey,
        createdAt: signed.created_at,
        rootEvent: issue.rootEvent,
        parentEvent: null,
        body: String(content.body ?? ""),
        requestId:
          typeof content.request_id === "string" ? content.request_id : null,
        import: null,
      });
    }
  }
}
