import type { DispatchExternalRef } from "./types";

export type DecisionOption = { id: string; label: string; consequence: string };
export type Milestone = {
  id: string;
  title: string;
  target_date: string | null;
  issue_ids: string[];
};
export type DeliveryStage =
  | "reported_complete"
  | "tested"
  | "reviewed"
  | "merged"
  | "installed"
  | "accepted";
export type AutomationControls = {
  service_id: string;
  service_pubkey: string;
  mode: "paused" | "preview" | "active";
  expires_at: number | null;
  max_actions_total: number;
  max_actions_per_tick: number;
  issue_ids: string[];
  capture_channels: string[];
  routing: { label: string; executor: string; channel: string }[];
  notify_triage: boolean;
  unassigned_after_secs: number | null;
  notification_channel: string;
};
export type CaptureAuthorization = {
  type: "capture";
  channel: string;
  workspace: string;
  project: string | null;
  enabled: boolean;
};
export type UnownedAlertAuthorization = {
  type: "unowned_alert";
  workspace: string;
  channel: string | null;
  after_hours: number;
  enabled: boolean;
};
export type AutopilotAuthorization = {
  type: "autopilot";
  workspace: string;
  project: string;
  agents: string[];
  label: string | null;
  at_a_time: number;
  per_day: number;
  reviewer_required: true;
  enabled: boolean;
  activation_id: string;
  confirmation_required: boolean;
  confirmation_event: string | null;
};
export type DispatchRecordPayload =
  | CaptureAuthorization
  | UnownedAlertAuthorization
  | AutopilotAuthorization
  | { type: "automation"; controls: AutomationControls }
  | { type: "view"; name: string; filters: Record<string, string> }
  | {
      type: "decision";
      issue_id: string;
      decider: string;
      question: string;
      context: string;
      options: DecisionOption[];
      evidence: DispatchExternalRef[];
      selected_option: string | null;
      rationale: string | null;
    }
  | {
      type: "project_plan";
      project_id: string;
      owner: string | null;
      purpose: string;
      outcome: string;
      target_date: string | null;
      milestones: Milestone[];
      links: DispatchExternalRef[];
    }
  | { type: "activation"; issue_id: string; active: boolean; reason: string }
  | {
      type: "delivery";
      issue_id: string;
      task_id: string | null;
      stage: DeliveryStage;
      artifact: string;
      description: string;
      passed: boolean;
      observed_at: number;
    }
  | {
      type: "capacity";
      executor: string;
      max_active: number;
      capabilities: string[];
      paused: boolean;
    };
export type DispatchRecord = {
  v: 1;
  record_id: string;
  revision: number;
  parent_event: string | null;
  request_id: string;
  archived: boolean;
  payload: DispatchRecordPayload;
};
export type DispatchRecordProjection = {
  creator: string;
  author: string;
  event_id: string;
  root_event: string;
  accepted_events?: string[];
  created_at: number;
  record: DispatchRecord;
};
export type PayloadOf<T extends DispatchRecordPayload["type"]> = Extract<
  DispatchRecordPayload,
  { type: T }
>;
export type RecordOf<T extends DispatchRecordPayload["type"]> =
  DispatchRecordProjection & {
    record: DispatchRecord & { payload: PayloadOf<T> };
  };
export function activeRecords<T extends DispatchRecordPayload["type"]>(
  records: DispatchRecordProjection[] | undefined,
  type: T,
): RecordOf<T>[] {
  const matching = (records ?? []).filter(
    (record) => record.record.payload.type === type,
  );
  if (["project_plan", "activation", "capacity", "automation"].includes(type)) {
    const latest = new Map<string, DispatchRecordProjection>();
    for (const value of matching) {
      const payload = value.record.payload;
      const key =
        payload.type === "automation"
          ? `${payload.controls.service_id}:${payload.controls.service_pubkey}`
          : payload.type === "project_plan"
            ? payload.project_id
            : payload.type === "activation"
              ? payload.issue_id
              : payload.type === "capacity"
                ? payload.executor
                : value.record.record_id;
      const previous = latest.get(key);
      if (
        !previous ||
        value.created_at > previous.created_at ||
        (value.created_at === previous.created_at &&
          value.event_id > previous.event_id)
      )
        latest.set(key, value);
    }
    return [...latest.values()].filter(
      (value) => !value.record.archived,
    ) as RecordOf<T>[];
  }
  return matching.filter((record) => !record.record.archived) as RecordOf<T>[];
}
