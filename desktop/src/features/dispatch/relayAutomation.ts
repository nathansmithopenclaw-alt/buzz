import { authorizationIntent } from "./relayAuthorizationIntent";
import { getJournalItem, setJournalItem } from "./journalStorage";
import { withLocalJournalLock } from "./localJournalLock";
import { stringify } from "yaml";
import { getRelayWsUrl } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import {
  createWorkflow,
  getChannelWorkflows,
  updateWorkflow,
} from "@/shared/api/tauriWorkflows";
import { assertDispatchAudience } from "./audience";
import {
  pendingPlanningChanges,
  publishPlanningRecord,
} from "./recordMutations";
import type {
  AutopilotAuthorization,
  CaptureAuthorization,
  UnownedAlertAuthorization,
  DispatchRecordProjection,
} from "./recordTypes";
import type { DispatchContext } from "./types";
import { captureWorkflowDefinition } from "./captureWorkflow";

export type RelayAuthorization =
  | AutopilotAuthorization
  | CaptureAuthorization
  | UnownedAlertAuthorization;
export function authorizationWorkflow(payload: RelayAuthorization) {
  if (payload.type === "capture")
    return captureWorkflowDefinition(payload.channel, payload.enabled);
  return {
    name:
      payload.type === "autopilot"
        ? `dispatch-autopilot-${payload.project}`
        : "dispatch-unowned-alert",
    enabled: payload.enabled,
    trigger: { on: "schedule", interval: "1m" },
    steps:
      payload.type === "autopilot"
        ? [{ id: "assign", action: "dispatch.assign" }]
        : [
            {
              id: "alert",
              action: "dispatch.escalate",
              after_hours: payload.after_hours,
            },
          ],
  };
}

/** Pair the workflow with its authority: revoke first on pause, authorize last on enable. */
async function saveRelayAuthorizationUnlocked({
  payload,
  previous,
  refresh,
  expectedScope,
}: {
  payload: RelayAuthorization;
  previous?: DispatchRecordProjection;
  refresh: () => Promise<DispatchContext>;
  expectedScope: { relay: string; signer: string };
}) {
  const stream =
    payload.type === "capture" ? payload.channel : payload.workspace;
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  if (relay !== expectedScope.relay || identity.pubkey !== expectedScope.signer)
    throw new Error(
      "Return to the original community and identity to retry this setting.",
    );
  const scope = {
    expectedRelayUrl: relay,
    expectedSignerPubkey: identity.pubkey,
  };
  if (payload.enabled && payload.type === "capture")
    await assertDispatchAudience(payload.channel, payload.workspace, [
      identity.pubkey,
    ]);
  if (payload.enabled && payload.type === "unowned_alert" && payload.channel)
    await assertDispatchAudience(payload.workspace, payload.channel, [
      identity.pubkey,
    ]);
  const definition = authorizationWorkflow(payload);
  const workflows = (await getChannelWorkflows(stream)).filter(
    (value) => value.name === definition.name,
  );
  if (workflows.length > 1)
    throw new Error(
      "Multiple workflows control this setting. Resolve them in Workflows before saving.",
    );
  const fresh = await refresh();
  if (!fresh.canAuthorizeAutomation)
    throw new Error("A current owner or admin must approve this setting.");
  const latest = fresh.records?.find((value) =>
    previous
      ? value.record.record_id === previous.record.record_id
      : !value.record.archived &&
        authorizationIntent(value.record.payload) ===
          authorizationIntent(payload),
  );
  // A pause may have committed its grant before its workflow write failed.
  // Recognize that exact already-accepted intent; never rebase a different edit.
  const alreadyAccepted =
    latest &&
    !latest.record.archived &&
    authorizationIntent(latest.record.payload) === authorizationIntent(payload);
  const pendingRecords = await pendingPlanningChanges(stream);
  const needsRecordRecovery = pendingRecords.some(
    (value) =>
      authorizationIntent(value.record.payload) ===
        authorizationIntent(payload) &&
      value.record.parent_event === (previous?.event_id ?? null),
  );
  const persist = () =>
    alreadyAccepted && !needsRecordRecovery
      ? Promise.resolve(latest)
      : publishPlanningRecord({
          workspace: stream,
          payload,
          previous,
          refresh,
          expectedScope,
        });
  // A failed second write remains visible and retryable. Never delete history.
  let accepted = payload.enabled ? undefined : await persist();
  const workflow = workflows[0];
  if (workflow)
    await updateWorkflow(
      workflow.id,
      stringify(definition),
      workflow.revision,
      scope,
    );
  else await createWorkflow(stream, stringify(definition), scope);
  if (payload.enabled) accepted = await persist();
  return accepted;
}

export function autopilotReviewLine(
  value: AutopilotAuthorization,
  agentNames: string[],
) {
  return `When an issue in this project is in Todo with no blockers${value.label ? ` and labeled ${value.label}` : ""}, Buzz assigns it to ${agentNames.length ? agentNames.join(" or ") : "the selected agent"}, at most ${value.at_a_time} at a time and ${value.per_day} per day. The issue owner reviews the result.`;
}

export function relaySettingKey(payload: RelayAuthorization) {
  return `${payload.type}:${payload.type === "autopilot" ? payload.project : payload.type === "capture" ? payload.channel : payload.workspace}`;
}
type PendingAuthorization = {
  payload: RelayAuthorization;
  previous?: DispatchRecordProjection;
};
const settingJournalKey = (
  relay: string,
  signer: string,
  stream: string,
  setting: string,
) =>
  `buzz-dispatch-relay-setting-v1:${encodeURIComponent(relay)}:${signer}:${stream}:${setting}`;
export async function pendingRelayAuthorization(
  stream: string,
  setting: string,
): Promise<PendingAuthorization | null> {
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  return readPending(
    settingJournalKey(relay, identity.pubkey, stream, setting),
    stream,
    setting,
  );
}
function readPending(
  key: string,
  stream: string,
  setting: string,
): PendingAuthorization | null {
  const raw = getJournalItem(key);
  if (!raw) return null;
  const value = JSON.parse(raw) as PendingAuthorization | null;
  if (
    value &&
    (!value.payload ||
      !["capture", "unowned_alert", "autopilot"].includes(value.payload.type) ||
      relaySettingKey(value.payload) !== setting ||
      (value.payload.type === "capture"
        ? value.payload.channel
        : value.payload.workspace) !== stream)
  )
    throw new Error(
      "The saved automation setting needs recovery before another change can be sent.",
    );
  return value;
}
/** Retain the entire paired save across window closure and uncertain acknowledgements. */
export async function saveRelayAuthorization(
  input: PendingAuthorization & { refresh: () => Promise<DispatchContext> },
) {
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  const stream =
    input.payload.type === "capture"
      ? input.payload.channel
      : input.payload.workspace;
  const setting = relaySettingKey(input.payload);
  const key = settingJournalKey(relay, identity.pubkey, stream, setting);
  return withLocalJournalLock(key, async () => {
    const pending = readPending(key, stream, setting);
    if (
      pending &&
      authorizationIntent(pending.payload) !==
        authorizationIntent(input.payload)
    )
      throw new Error(
        "Finish the saved automation change with Retry saved setting before making another change.",
      );
    const intent = pending ?? {
      payload: input.payload,
      previous: input.previous,
    };
    if (!setJournalItem(key, JSON.stringify(intent)))
      throw new Error(
        "Could not retain this setting for retry. Restore local storage before saving.",
      );
    const [currentIdentity, currentRelay] = await Promise.all([
      getIdentity(),
      getRelayWsUrl(),
    ]);
    if (currentIdentity.pubkey !== identity.pubkey || currentRelay !== relay)
      throw new Error(
        "Return to the original community to retry this saved setting.",
      );
    const result = await saveRelayAuthorizationUnlocked({
      ...intent,
      refresh: input.refresh,
      expectedScope: { relay, signer: identity.pubkey },
    });
    if (!setJournalItem(key, "null"))
      throw new Error(
        "Setting saved; its recovery receipt could not be cleared. Retry to confirm it.",
      );
    return result;
  });
}
