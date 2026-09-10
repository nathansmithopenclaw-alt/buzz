import { keepDispatchDraft, assertDraftScope } from "./dispatchDrafts";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { getRelayWsUrl, invokeTauri, signRelayEvent } from "@/shared/api/tauri";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { setJournalItem as setStorageItem } from "./journalStorage";
import { getJournalItem as getStorageItem } from "./journalStorage";
import { assertDispatchAudience } from "./audience";
import { withLocalJournalLock } from "./localJournalLock";
import { dispatchHistoryClient } from "./relayHistory";
import type { DispatchContext, DispatchLinkedTask } from "./types";

export type PendingTaskReview = {
  workspace: string;
  relay: string;
  signer: string;
  taskId: string;
  expectedEvent: string;
  event: RelayEvent;
  evidence: string;
  notes: string;
  accepted: boolean;
};
const journalKey = (
  relay: string,
  signer: string,
  workspace: string,
  task: string,
) =>
  `buzz-dispatch-task-review-v1:${encodeURIComponent(relay)}:${signer}:${workspace}:${task}`;
async function scope() {
  const [relay, identity] = await Promise.all([getRelayWsUrl(), getIdentity()]);
  return { relay, signer: identity.pubkey };
}
async function fence(value: { relay: string; signer: string }) {
  const current = await scope();
  if (current.relay !== value.relay || current.signer !== value.signer)
    throw new Error(
      "Return to the original community and identity to finish this review.",
    );
}
function read(key: string): PendingTaskReview | null {
  const raw = getStorageItem(key);
  if (!raw) return null;
  const value = JSON.parse(raw) as PendingTaskReview;
  if (
    value.event?.kind !== 9 ||
    value.event.pubkey !== value.signer ||
    !value.event.tags.some(
      (tag) =>
        tag[0] === "task" &&
        tag[1] === value.taskId &&
        tag[2] === (value.accepted ? "verified" : "failed"),
    )
  )
    throw new Error(
      "The saved review needs recovery before another review can be sent.",
    );
  return value;
}
export async function pendingTaskReview(workspace: string, taskId: string) {
  const current = await scope();
  return read(journalKey(current.relay, current.signer, workspace, taskId));
}
const taskAccepted = (context: DispatchContext, value: PendingTaskReview) =>
  context.linkedTasks.some(
    (task) =>
      task.taskId === value.taskId &&
      (task.eventId === value.event.id ||
        task.history?.some((entry) => entry.eventId === value.event.id)),
  );
async function resume(
  value: PendingTaskReview,
  refresh: () => Promise<DispatchContext>,
  key: string,
) {
  await fence(value);
  let current = await refresh();
  if (!taskAccepted(current, value)) {
    const task = current.linkedTasks.find(
      (task) => task.taskId === value.taskId,
    );
    if (
      current.sectionErrors.tasks ||
      current.viewerRole === "none" ||
      !task ||
      task.eventId !== value.expectedEvent
    )
      throw new Error(
        "The task changed or its history is unavailable. Your exact review is saved; compare it with the current result before retrying.",
      );
    await assertDispatchAudience(value.workspace, task.channelId);
    await fence(value);
    await relayClient.publishEvent(
      value.event,
      "Review confirmation is pending. The exact review is saved.",
      "The review could not be confirmed. Retry checks the same signed review.",
    );
    current = await refresh();
  }
  await fence(value);
  if (!taskAccepted(current, value))
    throw new Error(
      "The review is not yet in accepted history. Its exact signed event remains saved for recovery.",
    );
  if (!setStorageItem(key, ""))
    throw new Error(
      "The review was accepted, but its local recovery entry could not be cleared.",
    );
}
export async function retryTaskReview(
  value: PendingTaskReview,
  refresh: () => Promise<DispatchContext>,
) {
  const key = journalKey(
    value.relay,
    value.signer,
    value.workspace,
    value.taskId,
  );
  return withLocalJournalLock(key, async () => {
    if (read(key)?.event.id !== value.event.id)
      throw new Error("The saved review changed. Reopen it before retrying.");
    await resume(value, refresh, key);
  });
}
export async function reviewDispatchTask(input: {
  workspace: string;
  task: DispatchLinkedTask;
  accepted: boolean;
  evidence: string;
  notes: string;
  refresh: () => Promise<DispatchContext>;
}) {
  const initial = await scope(),
    key = journalKey(
      initial.relay,
      initial.signer,
      input.workspace,
      input.task.taskId,
    );
  return withLocalJournalLock(key, async () => {
    if (read(key))
      throw new Error(
        "A review is already saved for this attempt. Check it before creating another.",
      );
    const fresh = await input.refresh(),
      task = fresh.linkedTasks.find(
        (task) => task.taskId === input.task.taskId,
      );
    if (
      fresh.sectionErrors.tasks ||
      fresh.viewerRole === "none" ||
      !task ||
      task.eventId !== input.task.eventId ||
      task.status !== "done"
    )
      throw new Error(
        "The reported result changed. Reopen the current attempt before reviewing.",
      );
    await assertDispatchAudience(input.workspace, task.channelId);
    const history = dispatchHistoryClient();
    const [events, receipts, membership, workspaceMembership] =
      await Promise.all([
        history.fetchExhaustively({
          kinds: [9],
          "#h": [task.channelId],
          "#task": [task.taskId],
          "#t": ["buzz-task"],
        }),
        history.fetchExhaustively({ kinds: [40027], "#h": [task.channelId] }),
        history.fetch({ kinds: [39002], "#d": [task.channelId], limit: 1 }),
        history.fetch({ kinds: [39002], "#d": [input.workspace], limit: 1 }),
      ]);
    await fence(initial);
    if (!membership[0] || !workspaceMembership[0])
      throw new Error("Current review permissions are unavailable.");
    const shape = await invokeTauri<{
      kind: number;
      content: string;
      createdAt: number;
      tags: string[][];
    }>("build_dispatch_task_review_event", {
      channelId: task.channelId,
      events: [...events, ...receipts],
      membership: membership[0],
      workspaceMembership: workspaceMembership[0],
      authorPubkey: initial.signer,
      input: {
        task_id: task.taskId,
        expected_event: task.eventId,
        accepted: input.accepted,
        evidence: input.evidence,
        notes: input.notes,
      },
    });
    await fence(initial);
    const event = await signRelayEvent(shape);
    await fence(initial);
    if (event.pubkey !== initial.signer)
      throw new Error("The review identity changed before signing completed.");
    const value: PendingTaskReview = {
      ...initial,
      workspace: input.workspace,
      taskId: task.taskId,
      expectedEvent: task.eventId,
      event,
      accepted: input.accepted,
      evidence: input.evidence,
      notes: input.notes,
    };
    if (!setStorageItem(key, JSON.stringify(value)))
      throw new Error(
        "The review could not be saved for reliable recovery. Nothing was sent.",
      );
    await resume(value, input.refresh, key);
  });
}

export async function keepTaskReviewAsDraft(value: PendingTaskReview) {
  await assertDraftScope(value);
  const storageKey = journalKey(
    value.relay,
    value.signer,
    value.workspace,
    value.taskId,
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
        category: "Independent review",
        issueId: undefined,
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
