import { keepDispatchDraft, assertDraftScope } from "./dispatchDrafts";
import { withLocalJournalLock } from "./localJournalLock";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { getRelayWsUrl } from "@/shared/api/tauri";
import { relayClient } from "@/shared/api/relayClient";
import { setJournalItem as setStorageItem } from "./journalStorage";
import { getJournalItem as getStorageItem } from "./journalStorage";
import type { RelayEvent } from "@/shared/api/types";
import type { DispatchContext } from "./types";

export type PendingIssueWrite = {
  relay: string;
  signer: string;
  workspace: string;
  event: RelayEvent;
};
const key = (relay: string, signer: string) =>
  `buzz-dispatch-issue-outbox-v1:${encodeURIComponent(relay)}:${signer}`;
function read(storageKey: string): PendingIssueWrite[] {
  const raw = getStorageItem(storageKey);
  if (!raw) return [];
  const values: unknown = JSON.parse(raw);
  if (
    !Array.isArray(values) ||
    values.length > 50 ||
    values.some(
      (value) =>
        !value ||
        typeof value.relay !== "string" ||
        typeof value.signer !== "string" ||
        typeof value.workspace !== "string" ||
        !value.event ||
        ![40020, 40021, 40022].includes(value.event.kind) ||
        value.event.pubkey !== value.signer ||
        !Array.isArray(value.event.tags) ||
        !value.event.tags.some(
          (tag: string[]) => tag[0] === "h" && tag[1] === value.workspace,
        ),
    )
  )
    throw new Error(
      "The saved issue changes need recovery before another change can be sent.",
    );
  return values;
}
function write(storageKey: string, values: PendingIssueWrite[]) {
  if (values.length > 50 || !setStorageItem(storageKey, JSON.stringify(values)))
    throw new Error(
      "The change could not be saved for reliable retry. Free local storage and try again.",
    );
}
/** A repeat click may reuse only the same pending intent, scope and predecessor. */
export function equivalentPendingIssueWrite(a: RelayEvent, b: RelayEvent) {
  if (a.kind !== b.kind || a.pubkey !== b.pubkey) return false;
  const content = (event: RelayEvent) => {
    const value = JSON.parse(event.content);
    if (event.kind !== 40020) delete value.request_id;
    return value;
  };
  const tags = (event: RelayEvent) =>
    event.tags
      .filter(
        (tag) =>
          tag[0] !== "auth" && (event.kind === 40020 || tag[0] !== "request"),
      )
      .map((tag) => JSON.stringify(tag))
      .sort();
  return (
    JSON.stringify(content(a)) === JSON.stringify(content(b)) &&
    JSON.stringify(tags(a)) === JSON.stringify(tags(b))
  );
}
export async function retainIssueWrite(
  event: RelayEvent,
  expectedRelay: string,
): Promise<RelayEvent> {
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  if (identity.pubkey !== event.pubkey || relay !== expectedRelay)
    throw new Error(
      "Community or identity changed. Return to the original workspace before saving.",
    );
  const storageKey = key(relay, identity.pubkey);
  return withLocalJournalLock(storageKey, async () => {
    const values = read(storageKey);
    const previous = values.find((value) =>
      equivalentPendingIssueWrite(value.event, event),
    );
    if (previous) return previous.event;
    const workspace = event.tags.find((tag) => tag[0] === "h")?.[1];
    if (!workspace) throw new Error("The issue change has no workspace.");
    write(storageKey, [
      ...values,
      { relay, signer: identity.pubkey, workspace, event },
    ]);
    return event;
  });
}
export async function pendingIssueWrites(workspace: string) {
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  return read(key(relay, identity.pubkey)).filter(
    (value) => value.workspace === workspace,
  );
}
export async function publishRetainedIssueWrite(
  event: RelayEvent,
  timeout: string,
  failure: string,
  expectedRelay?: string,
) {
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  if (!expectedRelay || relay !== expectedRelay)
    throw new Error(
      "Return to the original community before publishing this saved change.",
    );
  const pending = read(key(expectedRelay, identity.pubkey)).find(
    (value) => value.event.id === event.id,
  );
  if (!pending || event.pubkey !== identity.pubkey)
    throw new Error(
      "Return to the original community and identity to publish the saved change.",
    );
  return relayClient.publishEvent(event, timeout, failure);
}
export async function acknowledgeIssueWrite(
  event: RelayEvent,
  expectedRelay?: string,
) {
  if (!expectedRelay)
    throw new Error("The saved change has no original relay scope.");
  const scope = { relay: expectedRelay, signer: event.pubkey };
  await assertDraftScope(scope);
  const storageKey = key(expectedRelay, event.pubkey);
  await withLocalJournalLock(storageKey, async () => {
    await assertDraftScope(scope);
    write(
      storageKey,
      read(storageKey).filter((value) => value.event.id !== event.id),
    );
  });
}
export function issueWriteAccepted(
  context: DispatchContext,
  event: RelayEvent,
) {
  return (
    context.activity.some((entry) => entry.eventId === event.id) ||
    context.comments.some((entry) => entry.eventId === event.id) ||
    context.issues.some(
      (issue) => issue.rootEvent === event.id || issue.tipEvent === event.id,
    )
  );
}
export async function retryIssueWrite(
  pending: PendingIssueWrite,
  refresh: () => Promise<DispatchContext>,
) {
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  if (relay !== pending.relay || identity.pubkey !== pending.signer)
    throw new Error("Return to the original community and identity to retry.");
  let context = await refresh();
  await assertDraftScope(pending);
  if (!issueWriteAccepted(context, pending.event)) {
    const body = JSON.parse(pending.event.content);
    const issue = context.issues.find(
      (issue) => issue.issueId === body.issue_id,
    );
    if (pending.event.kind === 40021 && issue?.tipEvent !== body.parent_event)
      throw new Error(
        "A newer issue version is present. Review the saved change against current history before resolving it.",
      );
    await publishRetainedIssueWrite(
      pending.event,
      "The exact change is retained for retry.",
      "Could not publish the saved change.",
      pending.relay,
    );
    context = await refresh();
  }
  if (!issueWriteAccepted(context, pending.event))
    throw new Error(
      "Sent, but the accepted issue history has not confirmed this change. It remains saved.",
    );
  await assertDraftScope(pending);
  await acknowledgeIssueWrite(pending.event, pending.relay);
}

export async function keepIssueWriteAsDraft(value: PendingIssueWrite) {
  await assertDraftScope(value);
  const storageKey = key(value.relay, value.signer);
  return withLocalJournalLock(storageKey, async () => {
    if (!read(storageKey).some((entry) => entry.event.id === value.event.id))
      throw new Error(
        "The saved change moved. Reload recovery before continuing.",
      );
    await keepDispatchDraft(
      {
        relay: value.relay,
        signer: value.signer,
        workspace: value.workspace,
        event: value.event,
        category: "Issue change",
        issueId: JSON.parse(value.event.content).issue_id,
      },
      () => {
        write(
          storageKey,
          read(storageKey).filter((entry) => entry.event.id !== value.event.id),
        );
      },
    );
  });
}
