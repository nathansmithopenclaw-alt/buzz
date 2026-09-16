import { keepDispatchDraft, assertDraftScope } from "./dispatchDrafts";
import { withLocalJournalLock } from "./localJournalLock";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { getRelayWsUrl, invokeTauri, signRelayEvent } from "@/shared/api/tauri";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { setJournalItem as setStorageItem } from "./journalStorage";
import { getJournalItem as getStorageItem } from "./journalStorage";
import type { DispatchContext } from "./types";
import type {
  DispatchRecord,
  DispatchRecordPayload,
  DispatchRecordProjection,
} from "./recordTypes";

export type PendingRecord = {
  workspace: string;
  relay: string;
  signer: string;
  record: DispatchRecord;
  event: RelayEvent;
};
function intentPayload(payload: DispatchRecordPayload) {
  return JSON.stringify(
    payload.type === "delivery" ? { ...payload, observed_at: 0 } : payload,
  );
}
const journalKey = (relay: string, signer: string, workspace: string) =>
  `buzz-dispatch-pending-v1:${encodeURIComponent(relay)}:${signer}:${workspace}`;
function readJournal(key: string): PendingRecord[] {
  const raw = getStorageItem(key);
  if (!raw) return [];
  const value: unknown = JSON.parse(raw);
  if (
    !Array.isArray(value) ||
    value.length > 20 ||
    value.some((entry) => {
      if (
        !entry ||
        typeof entry.workspace !== "string" ||
        typeof entry.relay !== "string" ||
        typeof entry.signer !== "string" ||
        !entry.record ||
        !entry.event ||
        entry.event.kind !== 40026 ||
        entry.event.pubkey !== entry.signer ||
        !Array.isArray(entry.event.tags) ||
        !entry.event.tags.some(
          (tag: string[]) => tag[0] === "h" && tag[1] === entry.workspace,
        )
      )
        return true;
      try {
        const signed = JSON.parse(entry.event.content);
        return (
          signed.record_id !== entry.record.record_id ||
          signed.request_id !== entry.record.request_id ||
          signed.parent_event !== entry.record.parent_event
        );
      } catch {
        return true;
      }
    })
  )
    throw new Error(
      "The saved change journal needs recovery before another change can be sent.",
    );
  return value as PendingRecord[];
}
function writeJournal(key: string, records: PendingRecord[]) {
  if (records.length > 20 || !setStorageItem(key, JSON.stringify(records)))
    throw new Error(
      "Buzz could not save this change for reliable retry. Free local storage and try again.",
    );
}
export async function pendingPlanningChanges(
  workspace: string,
): Promise<PendingRecord[]> {
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  return readJournal(journalKey(relay, identity.pubkey, workspace));
}

export async function retryPlanningChange(
  pending: PendingRecord,
  refresh: () => Promise<DispatchContext>,
) {
  return withLocalJournalLock(
    journalKey(pending.relay, pending.signer, pending.workspace),
    () => retryPlanningChangeUnlocked(pending, refresh),
  );
}

/** Retries the identical signed event; it cannot create another decision or revision. */
async function retryPlanningChangeUnlocked(
  pending: PendingRecord,
  refresh: () => Promise<DispatchContext>,
) {
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  if (
    pending.signer !== identity.pubkey ||
    pending.relay !== relay ||
    pending.event.pubkey !== identity.pubkey ||
    !pending.event.tags.some(
      (tag) => tag[0] === "h" && tag[1] === pending.workspace,
    )
  )
    throw new Error(
      "Return to the original community and identity to retry this change.",
    );
  const key = journalKey(relay, identity.pubkey, pending.workspace);
  if (!readJournal(key).some((value) => value.event.id === pending.event.id))
    throw new Error(
      "This retry entry changed. Reload recovery before continuing.",
    );
  const before = await refresh();
  const current = before.records?.find(
    (value) => value.record.record_id === pending.record.record_id,
  );
  if (
    current?.event_id !== pending.event.id &&
    !current?.accepted_events?.includes(pending.event.id)
  ) {
    const [stillIdentity, stillRelay] = await Promise.all([
      getIdentity(),
      getRelayWsUrl(),
    ]);
    if (stillRelay !== relay || stillIdentity.pubkey !== identity.pubkey)
      throw new Error("Community changed; the pending change was retained.");
    assertPlanningPermission(
      before,
      pending.signer,
      pending.record.payload,
      current,
    );
    if ((current?.event_id ?? null) !== pending.record.parent_event)
      throw new Error(
        "A newer shared record is present. Review the conflict before retrying the saved change.",
      );
    await relayClient.publishEvent(
      pending.event,
      "The change is awaiting relay confirmation. Retry sends this same change.",
      "Could not publish the saved change.",
    );
  }
  const after = await refresh();
  const accepted = after.records?.find(
    (value) =>
      value.event_id === pending.event.id ||
      value.accepted_events?.includes(pending.event.id),
  );
  if (!accepted)
    throw new Error(
      "The change was sent, but its current state is not confirmed. The exact signed change is saved for retry.",
    );
  await assertDraftScope(pending);
  writeJournal(
    key,
    readJournal(key).filter((value) => value.event.id !== pending.event.id),
  );
  return accepted;
}

/** One user commit becomes one validated immutable record with a durable retry receipt. */
async function publishPlanningRecordUnlocked({
  workspace,
  payload,
  previous,
  archived = false,
  refresh,
}: {
  workspace: string;
  payload: DispatchRecordPayload;
  previous?: DispatchRecordProjection;
  archived?: boolean;
  refresh: () => Promise<DispatchContext>;
}) {
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  const key = journalKey(relay, identity.pubkey, workspace);
  const pending = readJournal(key);
  const matching = pending.find(
    (value) =>
      value.record.parent_event === (previous?.event_id ?? null) &&
      intentPayload(value.record.payload) === intentPayload(payload) &&
      value.record.archived === archived,
  );
  if (matching) return retryPlanningChangeUnlocked(matching, refresh);
  if (
    previous &&
    pending.some(
      (value) => value.record.record_id === previous.record.record_id,
    )
  )
    throw new Error(
      "An earlier change to this record awaits confirmation. Retry it before making another change.",
    );
  const fresh = await refresh();
  assertPlanningPermission(fresh, identity.pubkey, payload, previous);
  if (
    previous &&
    fresh.records?.find(
      (value) => value.record.record_id === previous.record.record_id,
    )?.event_id !== previous.event_id
  )
    throw new Error(
      "This shared record changed. Reopen its latest version before saving.",
    );
  const record: DispatchRecord = {
    v: 1,
    record_id: previous?.record.record_id ?? crypto.randomUUID(),
    revision: (previous?.record.revision ?? -1) + 1,
    parent_event: previous?.event_id ?? null,
    request_id: crypto.randomUUID(),
    archived,
    payload,
  };
  const shape = await invokeTauri<{
    kind: number;
    content: string;
    createdAt: number;
    tags: string[][];
  }>("build_dispatch_record_event", {
    streamId: workspace,
    record,
    authorPubkey: identity.pubkey,
  });
  const [stillIdentity, stillRelay] = await Promise.all([
    getIdentity(),
    getRelayWsUrl(),
  ]);
  if (stillRelay !== relay || stillIdentity.pubkey !== identity.pubkey)
    throw new Error(
      "Community changed. Your draft is still available; return to its workspace to save.",
    );
  const event = await signRelayEvent({
    kind: shape.kind,
    content: shape.content,
    createdAt: shape.createdAt,
    tags: shape.tags,
    allowSelfTagging: true,
  });
  await assertDraftScope({ relay, signer: identity.pubkey });
  if (event.pubkey !== identity.pubkey)
    throw new Error("The signing identity changed before saving.");
  const entry = { workspace, relay, signer: identity.pubkey, record, event };
  writeJournal(key, [...readJournal(key), entry]);
  return retryPlanningChangeUnlocked(entry, refresh);
}

export async function publishPlanningRecord(
  input: Parameters<typeof publishPlanningRecordUnlocked>[0],
) {
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  return withLocalJournalLock(
    journalKey(relay, identity.pubkey, input.workspace),
    async () => {
      const [current, currentRelay] = await Promise.all([
        getIdentity(),
        getRelayWsUrl(),
      ]);
      if (currentRelay !== relay || current.pubkey !== identity.pubkey)
        throw new Error(
          "Community changed while this save was waiting. Your draft is retained.",
        );
      return publishPlanningRecordUnlocked(input);
    },
  );
}

export async function keepPlanningChangeAsDraft(value: PendingRecord) {
  await assertDraftScope(value);
  const storageKey = journalKey(value.relay, value.signer, value.workspace);
  return withLocalJournalLock(storageKey, async () => {
    if (
      !readJournal(storageKey).some(
        (entry) => entry.event.id === value.event.id,
      )
    )
      throw new Error(
        "The saved change moved. Reload recovery before continuing.",
      );
    await keepDispatchDraft(
      {
        relay: value.relay,
        signer: value.signer,
        workspace: value.workspace,
        event: value.event,
        category: "Shared planning change",
        issueId:
          "issue_id" in value.record.payload
            ? value.record.payload.issue_id
            : undefined,
      },
      () => {
        writeJournal(
          storageKey,
          readJournal(storageKey).filter(
            (entry) => entry.event.id !== value.event.id,
          ),
        );
      },
    );
  });
}

function assertPlanningPermission(
  context: DispatchContext,
  signer: string,
  payload: DispatchRecordPayload,
  previous?: DispatchRecordProjection,
) {
  if (context.sectionErrors.records || context.viewerRole === "none")
    throw new Error(
      context.sectionErrors.records ??
        "Current workspace membership is required.",
    );
  const maintainer = context.viewerRole === "maintainer";
  if (
    (["automation", "activation", "capacity"].includes(payload.type) ||
      (payload.type === "delivery" &&
        ["reviewed", "accepted"].includes(payload.stage))) &&
    !maintainer
  )
    throw new Error("A workspace maintainer must approve this control change.");
  const old = previous?.record.payload;
  if (
    old?.type === "decision" &&
    payload.type === "decision" &&
    (old.selected_option !== payload.selected_option ||
      old.selected_option !== null)
  ) {
    if (signer !== old.decider)
      throw new Error(
        "Only the named decider can resolve or revise this decision.",
      );
  } else if (previous && previous.creator !== signer && !maintainer)
    throw new Error(
      "Only the creator or a workspace maintainer can revise this shared record.",
    );
  if (
    !previous &&
    payload.type === "decision" &&
    payload.selected_option &&
    signer !== payload.decider
  )
    throw new Error("Only the named decider can record the chosen option.");
}
