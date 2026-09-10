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
import type { DispatchContext, DispatchIssue } from "./types";
import { parseMessageLink } from "@/features/messages/lib/messageLink";
import { activeRecords } from "./recordTypes";
export type NoticePurpose =
  | "tracking"
  | "status"
  | "blocked"
  | "completion"
  | "decision_requested"
  | "decision_resolved";
export type Notice = {
  workspace: string;
  issue: string;
  source_event: string | null;
  change_event: string;
  purpose: NoticePurpose;
};
export type UpdateChoice = {
  label: string;
  purpose: NoticePurpose;
  changeEvent: string;
  body: string;
  recipient?: string;
  sourceChannel?: string;
};
export function conversationTargets(issue: DispatchIssue) {
  const targets = new Map<string, { channel: string; source: string | null }>();
  for (const reference of issue.externalRefs) {
    const parsed = parseMessageLink(reference.ref);
    if (parsed.ok)
      targets.set(`${parsed.value.channelId}:${parsed.value.messageId}`, {
        channel: parsed.value.channelId,
        source: parsed.value.messageId,
      });
  }
  if (
    issue.homeChannelId &&
    ![...targets.values()].some(
      (value) => value.channel === issue.homeChannelId,
    )
  )
    targets.set(issue.homeChannelId, {
      channel: issue.homeChannelId,
      source: null,
    });
  return [...targets.values()];
}
export function conversationUpdateChoices(
  context: DispatchContext,
  issue: DispatchIssue,
): UpdateChoice[] {
  const choices: UpdateChoice[] = [
    {
      label: "Issue status",
      purpose: "status",
      changeEvent: issue.tipEvent,
      body: `Current issue status: ${issue.status.replaceAll("_", " ")}.`,
    },
  ];
  for (const task of context.linkedTasks
    .filter((task) => task.issueId === issue.issueId)
    .sort((a, b) => b.createdAt - a.createdAt)) {
    if (task.status === "blocked")
      choices.push({
        label: "Execution blocker",
        purpose: "blocked",
        changeEvent: task.eventId,
        sourceChannel: task.channelId,
        body: `Execution is blocked.\n\n${task.blockedOn ?? "Open the attempt to inspect the blocker."}`,
      });
    if (["done", "verified", "failed"].includes(task.status))
      choices.push({
        label:
          task.status === "verified"
            ? "Accepted review"
            : task.status === "done"
              ? "Reported completion"
              : "Failed attempt",
        purpose: "completion",
        changeEvent: task.eventId,
        sourceChannel: task.channelId,
        body: `${task.status === "verified" ? "The execution result received independent review acceptance." : task.status === "done" ? "The executor reports completion. Independent acceptance is pending." : "This execution attempt ended unsuccessfully."}\n\n${task.notes ?? ""}${task.verification?.evidence_ref ? `\n\nEvidence: ${task.verification.evidence_ref}` : ""}`,
      });
  }
  for (const decision of activeRecords(context.records, "decision").filter(
    (value) => value.record.payload.issue_id === issue.issueId,
  )) {
    const value = decision.record.payload;
    choices.push(
      value.selected_option
        ? {
            label: "Decision resolved",
            purpose: "decision_resolved",
            changeEvent: decision.event_id,
            body: `${value.question}\n\nChoice: ${value.options.find((option) => option.id === value.selected_option)?.label ?? value.selected_option}\n\nRationale: ${value.rationale ?? ""}`,
          }
        : {
            label: "Request a decision",
            purpose: "decision_requested",
            changeEvent: decision.event_id,
            body: `Decision needed: ${value.question}\n\nReview the options, consequences and evidence in Dispatch.`,
            recipient: value.decider,
          },
    );
  }
  return choices;
}
export type PendingConversationUpdate = {
  relay: string;
  signer: string;
  channel: string;
  notice: Notice;
  event: RelayEvent;
};
const key = (relay: string, signer: string, workspace: string, issue: string) =>
  `buzz-dispatch-conversation-v1:${encodeURIComponent(relay)}:${signer}:${workspace}:${issue}`;
const tag = (event: RelayEvent, name: string) =>
  event.tags.find((tag) => tag[0] === name)?.[1];
async function scope() {
  const [relay, identity] = await Promise.all([getRelayWsUrl(), getIdentity()]);
  return { relay, signer: identity.pubkey };
}
async function fence(value: { relay: string; signer: string }) {
  const current = await scope();
  if (current.relay !== value.relay || current.signer !== value.signer)
    throw new Error(
      "Return to the original community and identity before sharing this update.",
    );
}
function read(storageKey: string): PendingConversationUpdate | null {
  const raw = getStorageItem(storageKey);
  if (!raw) return null;
  const value = JSON.parse(raw) as PendingConversationUpdate;
  if (
    value.event?.kind !== 9 ||
    value.event.pubkey !== value.signer ||
    tag(value.event, "h") !== value.channel ||
    tag(value.event, "t") !== "dispatch-update" ||
    !tag(value.event, "d")
  )
    throw new Error("The saved conversation update needs local recovery.");
  return value;
}
export async function pendingConversationUpdate(
  workspace: string,
  issue: string,
) {
  const current = await scope();
  return read(key(current.relay, current.signer, workspace, issue));
}
/** Current projection through the same read-only SDK command used by mobile. */
export async function readDispatchIssueSnapshot(
  workspace: string,
  issue: string,
  expectedScope?: { relay: string; signer: string },
) {
  if (expectedScope) await fence(expectedScope);
  const initial = await scope();
  const event = await signRelayEvent({
    kind: 40028,
    content: JSON.stringify({
      v: 1,
      request: { operation: "snapshot", issue_id: issue, include_closed: true },
    }),
    tags: [
      ["h", workspace],
      ["request", crypto.randomUUID()],
    ],
  });
  await fence(initial);
  const reply = await relayClient.readDispatchCommand(
    event,
    "Dispatch read timed out.",
    "Current Dispatch state could not be read.",
  );
  await fence(initial);
  if (!reply.startsWith("response:"))
    throw new Error(
      "This relay needs the Dispatch update before it can confirm current work here.",
    );
  const value = JSON.parse(reply.slice(9));
  if (
    value.v !== 1 ||
    value.workspace !== workspace ||
    !Array.isArray(value.issues)
  )
    throw new Error("The relay returned an incompatible Dispatch snapshot.");
  return value as {
    v: number;
    workspace: string;
    capabilities?: { dispatchNotice?: number };
    issues: {
      issueId: string;
      title: string;
      identifier: string;
      status: string;
    }[];
  };
}
async function findShared(value: PendingConversationUpdate) {
  const found = await dispatchHistoryClient().fetchExhaustively({
    kinds: [9],
    "#h": [value.channel],
    "#t": ["dispatch-update"],
    "#d": [tag(value.event, "d") ?? ""],
  });
  if (found.length > 1)
    throw new Error(
      "More than one historical update has this identity. Review the conversation before sending anything else.",
    );
  return found[0]
    ? invokeTauri<RelayEvent>("validate_dispatch_notice_readback", {
        event: found[0],
        channelId: value.channel,
        notice: value.notice,
      })
    : null;
}
async function resume(
  value: PendingConversationUpdate,
  refresh: () => Promise<DispatchContext>,
  storageKey: string,
) {
  await fence(value);
  let accepted = await findShared(value);
  if (!accepted) {
    const snapshot = await readDispatchIssueSnapshot(
      value.notice.workspace,
      value.notice.issue,
    );
    if (snapshot.capabilities?.dispatchNotice !== 1)
      throw new Error(
        "The relay must support duplicate-safe Dispatch updates before this message can be sent. Your draft remains saved.",
      );
    const current = await refresh(),
      issue = current.issues.find(
        (issue) => issue.issueId === value.notice.issue,
      );
    const choice = issue
      ? conversationUpdateChoices(current, issue).find(
          (choice) =>
            choice.changeEvent === value.notice.change_event &&
            choice.purpose === value.notice.purpose,
        )
      : null;
    if (!issue || current.viewerRole === "none" || !choice)
      throw new Error(
        "The work changed after this update was prepared. Keep the draft and review its current status before sharing.",
      );
    const recipient = tag(value.event, "p");
    await assertDispatchAudience(
      value.notice.workspace,
      value.channel,
      recipient ? [recipient] : [],
    );
    if (choice.sourceChannel && choice.sourceChannel !== value.notice.workspace)
      await assertDispatchAudience(choice.sourceChannel, value.channel);
    await fence(value);
    try {
      await relayClient.publishEvent(
        value.event,
        "Update confirmation is pending; the exact message is saved.",
        "The saved update was not confirmed.",
      );
    } catch (error) {
      accepted = await findShared(value);
      if (!accepted) throw error;
    }
    accepted ??= await findShared(value);
  }
  await fence(value);
  if (!accepted)
    throw new Error(
      "The update has not appeared in the conversation. Retry checks the same signed message.",
    );
  if (!setStorageItem(storageKey, ""))
    throw new Error(
      "The update is shared, but its local recovery copy could not be cleared.",
    );
  return accepted;
}
export async function retryConversationUpdate(
  value: PendingConversationUpdate,
  refresh: () => Promise<DispatchContext>,
) {
  const storageKey = key(
    value.relay,
    value.signer,
    value.notice.workspace,
    value.notice.issue,
  );
  return withLocalJournalLock(storageKey, async () => {
    if (read(storageKey)?.event.id !== value.event.id)
      throw new Error("The saved update changed. Reload recovery.");
    return resume(value, refresh, storageKey);
  });
}
export async function shareConversationUpdate(input: {
  expectedScope: { relay: string; signer: string };
  notice: Notice;
  channel: string;
  body: string;
  recipient?: string;
  refresh: () => Promise<DispatchContext>;
}) {
  await fence(input.expectedScope);
  const initial = await scope(),
    storageKey = key(
      initial.relay,
      initial.signer,
      input.notice.workspace,
      input.notice.issue,
    );
  return withLocalJournalLock(storageKey, async () => {
    if (read(storageKey))
      throw new Error(
        "A conversation update is already saved. Check it before composing another.",
      );
    await assertDispatchAudience(
      input.notice.workspace,
      input.channel,
      input.recipient ? [input.recipient] : [],
    );
    const source = input.notice.source_event
      ? (
          await dispatchHistoryClient().fetch({
            kinds: [9],
            ids: [input.notice.source_event],
            "#h": [input.channel],
            limit: 1,
          })
        )[0]
      : null;
    await fence(initial);
    const shape = await invokeTauri<{
      kind: number;
      content: string;
      createdAt: number;
      tags: string[][];
    }>("build_dispatch_notice_event", {
      channelId: input.channel,
      notice: input.notice,
      body: input.body,
      source: source ?? null,
      mentions: input.recipient ? [input.recipient] : [],
      authorPubkey: initial.signer,
    });
    const event = await signRelayEvent(shape);
    await fence(initial);
    if (event.pubkey !== initial.signer)
      throw new Error(
        "The signing identity changed. Reopen the original draft.",
      );
    const value = {
      ...initial,
      channel: input.channel,
      notice: input.notice,
      event,
    };
    if (!setStorageItem(storageKey, JSON.stringify(value)))
      throw new Error(
        "The update could not be retained for safe retry. Nothing was sent.",
      );
    return resume(value, input.refresh, storageKey);
  });
}

export async function keepConversationUpdateAsDraft(
  value: PendingConversationUpdate,
) {
  await assertDraftScope(value);
  const storageKey = key(
    value.relay,
    value.signer,
    value.notice.workspace,
    value.notice.issue,
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
        workspace: value.notice.workspace,
        event: value.event,
        category: "Conversation update",
        issueId: value.notice.issue,
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
