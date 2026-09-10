import { assignmentWarnings } from "./attention";
import { keepDispatchDraft, assertDraftScope } from "./dispatchDrafts";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { getRelayWsUrl, invokeTauri, signRelayEvent } from "@/shared/api/tauri";
import { relayClient } from "@/shared/api/relayClient";
import { setJournalItem as setStorageItem } from "./journalStorage";
import { getJournalItem as getStorageItem } from "./journalStorage";
import { assertDispatchAudience } from "./audience";
import { withLocalJournalLock } from "./localJournalLock";
import { publishDispatchIssueMutation } from "./mutations";
import { dispatchContextExport } from "./contextExport";
import type { DispatchContext, DispatchIssue } from "./types";
import type { RelayEvent } from "@/shared/api/types";
export type PendingAssignment = {
  relay: string;
  signer: string;
  workspace: string;
  issueId: string;
  sourceTip: string;
  sourceChannels: string[];
  routingWarnings: string[];
  overrideReason: string;
  linkEventId?: string;
  taskId: string;
  executor: string;
  reviewer: string;
  channel: string;
  event: RelayEvent;
};
const key = (relay: string, signer: string, workspace: string, issue: string) =>
  `buzz-dispatch-assignment-v1:${encodeURIComponent(relay)}:${signer}:${workspace}:${issue}`;
function read(storageKey: string): PendingAssignment | null {
  const raw = getStorageItem(storageKey);
  if (!raw) return null;
  const value = JSON.parse(raw) as PendingAssignment;
  const fence = "```buzz-task\n";
  const content = value.event?.content ?? "";
  const envelope = JSON.parse(
    content
      .slice(content.lastIndexOf(fence) + fence.length)
      .trim()
      .replace(/\n```$/, ""),
  );
  if (
    envelope.task_id !== value.taskId ||
    envelope.assignee !== value.executor ||
    envelope.assigner !== value.signer ||
    envelope.authority?.reviewer !== value.reviewer ||
    envelope.tracker_issue_id !== value.issueId ||
    envelope.status !== "assigned" ||
    value.event?.kind !== 9 ||
    value.event.pubkey !== value.signer ||
    !value.event.tags?.some(
      (tag) =>
        tag[0] === "task" && tag[1] === value.taskId && tag[2] === "assigned",
    ) ||
    !value.event.tags.some((tag) => tag[0] === "h" && tag[1] === value.channel)
  )
    throw new Error(
      "The saved assignment needs local recovery before a new attempt can start.",
    );
  return value;
}
async function scope() {
  const [relay, identity] = await Promise.all([getRelayWsUrl(), getIdentity()]);
  return { relay, signer: identity.pubkey };
}
export async function pendingAssignment(workspace: string, issueId: string) {
  const current = await scope();
  return read(key(current.relay, current.signer, workspace, issueId));
}
async function assertScope(value: Pick<PendingAssignment, "relay" | "signer">) {
  const current = await scope();
  if (current.relay !== value.relay || current.signer !== value.signer)
    throw new Error(
      "Return to the original community and identity before continuing this assignment.",
    );
}
export async function assignDispatchIssue(input: {
  issue: DispatchIssue;
  executor: string;
  reviewer: string;
  channel: string;
  method: string;
  overrideReason: string;
  refresh: () => Promise<DispatchContext>;
}) {
  const initial = await scope(),
    storageKey = key(
      initial.relay,
      initial.signer,
      input.issue.streamId,
      input.issue.issueId,
    );
  return withLocalJournalLock(storageKey, async () => {
    if (read(storageKey))
      throw new Error(
        "An assignment is already saved for this issue. Review and retry it before creating another attempt.",
      );
    if (input.executor === input.reviewer)
      throw new Error(
        "The executor and independent reviewer must be different people.",
      );
    const current = await input.refresh(),
      issue = current.issues.find(
        (value) => value.issueId === input.issue.issueId,
      );
    if (!issue || issue.tipEvent !== input.issue.tipEvent)
      throw new Error(
        "This issue changed. Review its current scope before assigning.",
      );
    if (current.viewerRole === "none" || current.sectionErrors.tasks)
      throw new Error(
        "Current membership and complete task history are required.",
      );
    if (
      current.linkedTasks.some(
        (task) =>
          task.issueId === issue.issueId &&
          !["failed", "verified"].includes(task.status),
      )
    )
      throw new Error(
        "An attempt is active or awaiting review. Resolve it before starting another.",
      );
    if (
      ["done", "cancelled"].includes(issue.status) ||
      current.sectionErrors.records
    )
      throw new Error(
        "Open issue state and current routing controls are required before assigning.",
      );
    const warnings = assignmentWarnings(current, issue, input.executor);
    if (warnings.length && !input.overrideReason.trim())
      throw new Error(
        `Review these routing conditions and explain an override: ${warnings.join("; ")}`,
      );
    if (input.overrideReason.length > 2000)
      throw new Error("Keep the routing explanation under 2,000 characters.");
    const unresolvedLink = issue.externalRefs.some((ref) =>
      ref.type === "task"
        ? !current.linkedTasks.some(
            (task) => task.issueId === issue.issueId && task.taskId === ref.ref,
          )
        : ref.type === "task_event" &&
          !current.linkedTasks.some(
            (task) =>
              task.issueId === issue.issueId &&
              (task.eventId === ref.ref ||
                task.assignmentEventId === ref.ref ||
                task.history?.some((entry) => entry.eventId === ref.ref)),
          ),
    );
    if (unresolvedLink)
      throw new Error(
        "An existing execution link has no accepted task history. Reconcile that link before starting another attempt; an earlier assignment may still arrive.",
      );
    await assertDispatchAudience(issue.streamId, input.channel, [
      input.executor,
      input.reviewer,
    ]);
    await assertScope(initial);
    const taskId = crypto.randomUUID(),
      requestId = crypto.randomUUID();
    const sourceChannels = [
      ...new Set([
        issue.streamId,
        ...current.linkedTasks
          .filter((task) => task.issueId === issue.issueId)
          .map((task) => task.channelId),
      ]),
    ];
    for (const channel of sourceChannels) {
      if (channel !== issue.streamId)
        await assertDispatchAudience(channel, input.channel, [
          input.executor,
          input.reviewer,
        ]);
    }
    await assertScope(initial);
    const brief = dispatchContextExport(current, issue, 0, 24000);
    const envelope = {
      v: 3,
      authority: {
        assignment_event_id: null,
        previous_event_id: null,
        reviewer: input.reviewer,
      },
      task_id: taskId,
      status: "assigned",
      assignee: input.executor,
      assigner: initial.signer,
      title: issue.title,
      deliverables: [],
      verification: { method: input.method, evidence_ref: null },
      blocked_on: null,
      notes: input.overrideReason
        ? `Human routing override: ${input.overrideReason}`
        : "Explicit assignment from Dispatch",
      tracker_issue_id: issue.issueId,
      dispatch_request_id: requestId,
    };
    const unsigned = await invokeTauri<{
      kind: number;
      content: string;
      createdAt: number;
      tags: string[][];
    }>("build_dispatch_assignment_event", {
      channelId: input.channel,
      envelope,
      context: `**${issue.title}**\n\nAccepted scope and evidence:\n\n${brief}`,
      authorPubkey: initial.signer,
    });
    const event = await signRelayEvent(unsigned);
    await assertScope(initial);
    if (event.pubkey !== initial.signer)
      throw new Error(
        "The signing identity changed before the assignment was saved.",
      );
    const pending: PendingAssignment = {
      ...initial,
      workspace: issue.streamId,
      issueId: issue.issueId,
      sourceTip: issue.tipEvent,
      sourceChannels,
      routingWarnings: warnings,
      overrideReason: input.overrideReason.trim(),
      taskId,
      executor: input.executor,
      reviewer: input.reviewer,
      channel: input.channel,
      event,
    };
    if (!setStorageItem(storageKey, JSON.stringify(pending)))
      throw new Error(
        "Could not retain the assignment for safe retry. No work was sent.",
      );
    return resume(pending, input.refresh, storageKey);
  });
}
export async function retryAssignment(
  pending: PendingAssignment,
  refresh: () => Promise<DispatchContext>,
) {
  await assertScope(pending);
  const storageKey = key(
    pending.relay,
    pending.signer,
    pending.workspace,
    pending.issueId,
  );
  return withLocalJournalLock(storageKey, async () => {
    const saved = read(storageKey);
    if (saved?.event.id !== pending.event.id)
      throw new Error(
        "The saved assignment changed. Reload it before continuing.",
      );
    return resume(pending, refresh, storageKey);
  });
}
async function resume(
  pending: PendingAssignment,
  refresh: () => Promise<DispatchContext>,
  storageKey: string,
) {
  await assertScope(pending);
  let current = await refresh(),
    issue = current.issues.find((value) => value.issueId === pending.issueId);
  if (!issue || current.viewerRole === "none")
    throw new Error(
      "The original issue and workspace membership are required to resume.",
    );
  const accepted = () =>
    current.linkedTasks.some(
      (task) =>
        task.taskId === pending.taskId &&
        (task.eventId === pending.event.id ||
          task.assignmentEventId === pending.event.id),
    );
  if (!accepted()) {
    if (current.sectionErrors.tasks || current.sectionErrors.records)
      throw new Error(
        "Complete task history and current routing controls are required before resuming.",
      );
    const warnings = assignmentWarnings(current, issue, pending.executor);
    if (
      warnings.some((warning) => !pending.routingWarnings?.includes(warning)) ||
      (warnings.length && !pending.overrideReason)
    )
      throw new Error(
        "Routing conditions changed after preparation. Keep the assignment as a draft and reconcile its link before preparing a new attempt.",
      );
    await assertAssignmentAudience(pending);
    await assertScope(pending);
    if (
      !issue.externalRefs.some(
        (ref) => ref.type === "task_event" && ref.ref === pending.event.id,
      )
    ) {
      if (issue.tipEvent !== pending.sourceTip)
        throw new Error(
          "The issue changed before its task link was accepted. Review the saved assignment against the current issue.",
        );
      await publishDispatchIssueMutation({
        issue,
        mutation: {
          type: "edit",
          expectedTip: pending.sourceTip,
          fields: {
            externalRefs: [
              ...issue.externalRefs,
              { type: "task", ref: pending.taskId },
              { type: "task_event", ref: pending.event.id },
            ],
          },
          reason:
            "Linked a human-reviewed execution attempt; issue owner retained",
        },
        viewerRole: current.viewerRole,
        onOptimistic: () => {},
        onRollback: () => {},
        refresh,
      });
      current = await refresh();
      issue = current.issues.find((value) => value.issueId === pending.issueId);
    }
    if (
      !issue?.externalRefs.some(
        (ref) => ref.type === "task_event" && ref.ref === pending.event.id,
      )
    )
      throw new Error(
        "The issue link is not in accepted history. The assignment is retained and no executor was notified.",
      );
    if (["done", "cancelled"].includes(issue.status))
      throw new Error(
        "The issue closed before execution started. The assignment remains saved for review.",
      );
    const linkEntry = current.activity.find(
      (entry) =>
        entry.issueId === pending.issueId &&
        entry.changes.some(
          (change) =>
            ["externalRefs", "external_refs"].includes(change.field) &&
            Array.isArray(change.after) &&
            change.after.some(
              (value: unknown) =>
                typeof value === "object" &&
                value !== null &&
                "type" in value &&
                "ref" in value &&
                value.type === "task_event" &&
                value.ref === pending.event.id,
            ),
        ),
    );
    const linkEventId = pending.linkEventId ?? linkEntry?.eventId;
    if (!linkEventId || issue.tipEvent !== linkEventId)
      throw new Error(
        "The issue changed after its assignment was prepared. Review the saved scope before notifying the executor.",
      );
    if (!pending.linkEventId) {
      pending.linkEventId = linkEventId;
      if (!setStorageItem(storageKey, JSON.stringify(pending)))
        throw new Error(
          "The accepted task link could not be retained locally. The executor has not been notified.",
        );
    }
    await assertScope(pending);
    await assertAssignmentAudience(pending);
    await assertScope(pending);
    await relayClient.publishEvent(
      pending.event,
      "The saved assignment is awaiting confirmation.",
      "The assignment was not confirmed; its exact signed event is saved for retry.",
    );
    current = await refresh();
  }
  await assertScope(pending);
  if (!accepted())
    throw new Error(
      "Published, but the signed assignment has not appeared in accepted task history. Retry checks this same event.",
    );
  if (!setStorageItem(storageKey, ""))
    throw new Error(
      "Assignment confirmed, but its local retry entry could not be cleared. A retry will check the accepted event.",
    );
  return pending.taskId;
}

export async function keepAssignmentAsDraft(value: PendingAssignment) {
  await assertDraftScope(value);
  const storageKey = key(
    value.relay,
    value.signer,
    value.workspace,
    value.issueId,
  );
  return withLocalJournalLock(storageKey, async () => {
    if (!(read(storageKey)?.event.id === value.event.id))
      throw new Error(
        "The saved change moved. Reload recovery before continuing.",
      );
    await keepDispatchDraft(
      {
        relay: value.relay,
        signer: value.signer,
        workspace: value.workspace,
        event: value.event,
        category: "Execution assignment",
        issueId: value.issueId,
      },
      () => {
        if (!setStorageItem(storageKey, ""))
          throw new Error(
            "The draft is saved, but its retry entry could not be cleared. Keep it as a draft again to finish recovery.",
          );
      },
    );
  });
}

async function assertAssignmentAudience(pending: PendingAssignment) {
  if (
    !Array.isArray(pending.sourceChannels) ||
    !pending.sourceChannels.includes(pending.workspace)
  )
    throw new Error(
      "This older assignment has no saved audience inventory. Keep it as a draft, reconcile its issue link, and review current context before preparing another.",
    );
  for (const channel of new Set(pending.sourceChannels)) {
    await assertDispatchAudience(channel, pending.channel, [
      pending.executor,
      pending.reviewer,
    ]);
    await assertScope(pending);
  }
}
