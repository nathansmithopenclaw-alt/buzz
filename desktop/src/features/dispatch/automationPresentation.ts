import {
  activeRecords,
  type AutomationControls,
  type RecordOf,
} from "./recordTypes";
import type { DispatchContext } from "./types";

export type AutomationTask = "capture" | "remind" | "assign";
export const AUTOMATION_TASKS = [
  {
    id: "capture",
    title: "Save messages as work",
    description:
      "Add 📌 to a message. Buzz creates an issue with a link back to the conversation.",
    example: "A good idea in #design becomes something you can follow up on.",
  },
  {
    id: "remind",
    title: "Remind us about unassigned work",
    description:
      "Post a reminder when selected issues need someone to take ownership.",
    example: "An issue has no owner after a day. Your team gets a reminder.",
  },
  {
    id: "assign",
    title: "Send work to an agent",
    description:
      "Send selected issues to an agent when they are ready and match a label you choose.",
    example:
      "An issue labeled “research” is ready. Your research agent picks it up.",
  },
] as const;

export function automationTasks(
  controls: AutomationControls,
): AutomationTask[] {
  return [
    ...(controls.capture_channels.length ? ["capture" as const] : []),
    ...(controls.notify_triage || controls.unassigned_after_secs !== null
      ? ["remind" as const]
      : []),
    ...(controls.routing.length ? ["assign" as const] : []),
  ];
}
export function automationTitle(controls: AutomationControls) {
  const tasks = automationTasks(controls);
  return tasks.length === 1
    ? (AUTOMATION_TASKS.find((task) => task.id === tasks[0])?.title ??
        "Follow up on work")
    : tasks.length
      ? "Follow up on work"
      : "Choose what Buzz should do";
}
export function initialAutomation(
  workspace: string,
  previous?: AutomationControls,
): AutomationControls {
  return previous
    ? { ...previous, mode: "preview", expires_at: null }
    : {
        service_id: "",
        service_pubkey: "",
        mode: "preview",
        expires_at: null,
        max_actions_total: 3,
        max_actions_per_tick: 1,
        issue_ids: [],
        capture_channels: [],
        routing: [],
        notify_triage: false,
        unassigned_after_secs: null,
        notification_channel: workspace,
      };
}

/** A current, matching acknowledgement is needed before presenting work as enabled. */
export function automationState(
  context: DispatchContext,
  value: RecordOf<"automation">,
  now: number,
) {
  const controls = value.record.payload.controls;
  const beat = context.automation?.heartbeats.find(
    (item) =>
      item.serviceId === controls.service_id &&
      item.signer === controls.service_pubkey,
  );
  const fresh = Boolean(
    beat && now - beat.observedAt < 300 && beat.observedAt <= now + 300,
  );
  let detail: Record<string, unknown> = {};
  try {
    const parsed: unknown = JSON.parse(beat?.health.detail ?? "null");
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed))
      detail = parsed as Record<string, unknown>;
  } catch {
    /* Unstructured diagnostic text is available in technical details. */
  }
  const acknowledged =
    fresh &&
    beat?.health.status === "healthy" &&
    detail.approval === value.event_id &&
    detail.controls === controls.mode;
  const count = (key: string) =>
    acknowledged &&
    typeof detail[key] === "number" &&
    Number.isSafeInteger(detail[key]) &&
    (detail[key] as number) >= 0
      ? (detail[key] as number)
      : null;
  const used = count("spent"),
    pending = count("pending");
  const remaining =
    used === null ? null : Math.max(0, controls.max_actions_total - used);
  const permissionReady = Boolean(
    context.admission &&
      context.admission.unsealed === 0 &&
      context.admission.diagnostics.length === 0 &&
      !context.sectionErrors.records,
  );
  const hasWork = Boolean(
    controls.capture_channels.length ||
      (controls.issue_ids.length &&
        (controls.routing.length ||
          controls.notify_triage ||
          controls.unassigned_after_secs)),
  );
  const expired = controls.expires_at !== null && controls.expires_at <= now;
  let label = "Off",
    explanation = "Save a preview, then choose when to turn it on.";
  if (!beat) {
    label = "Waiting for connection";
    explanation =
      "Buzz has saved your settings, but automation has not connected yet. Ask your workspace administrator to check the connection.";
  } else if (!fresh) {
    label = "Connection lost";
    explanation =
      "Buzz cannot confirm what is happening right now. Check your connection or ask your workspace administrator for help.";
  } else if (beat.health.status === "degraded") {
    label = "Needs attention";
    const diagnostic = beat.health.detail ?? "";
    explanation = diagnostic.includes("capture channels exceed")
      ? "This connection cannot watch one of the channels you chose. Ask your workspace administrator to allow that channel, or edit the setup to choose another."
      : diagnostic.includes("notice channel differs")
        ? "This connection cannot post reminders in the channel you chose. Ask your workspace administrator to allow it, or choose another reminder channel."
        : diagnostic.includes("routing exceeds")
          ? "This connection cannot send work to the agent or channel you chose. Ask your workspace administrator to allow this assignment, or edit the setup."
          : "Buzz could not finish checking this setup. Ask your workspace administrator to check the connection details below.";
  } else if (!acknowledged) {
    label = controls.mode === "paused" ? "Pausing…" : "Checking setup…";
    explanation =
      "Waiting for Buzz to confirm your latest settings. Work already sent to an agent may continue.";
  } else if (controls.mode === "paused") {
    label = "Paused";
    explanation =
      "No new work will be started by this automation. Work already sent to an agent can continue.";
  } else if (controls.mode === "preview" && remaining === 0) {
    label = "Limit reached";
    explanation =
      "This run has used its limit. Review another run before doing more.";
  } else if (
    controls.mode === "preview" &&
    (!permissionReady || used === null || pending !== 0)
  ) {
    label = "Checking setup…";
    explanation =
      "Buzz is confirming the available actions and workspace access before this can start.";
  } else if (controls.mode === "preview" && !hasWork) {
    label = "Choose a job";
    explanation = "Edit this setup to choose the work Buzz should help with.";
  } else if (controls.mode === "preview") {
    label = "Ready to turn on";
    explanation = "Your preview is saved. Nothing runs until you turn it on.";
  } else if (expired) {
    label = "Time is up";
    explanation =
      "The one-hour run has ended. Review the results before starting another run.";
  } else if (remaining === 0) {
    label = pending ? "Finishing up" : "Limit reached";
    explanation = pending
      ? "The limit has been reached. Buzz is checking work it already started."
      : "Buzz has used this run’s limit. Review the results before starting another run.";
  } else {
    label = "On";
    explanation =
      "Buzz is checking the work you selected and will stop at the time or limit shown below.";
  }
  const canStart =
    controls.mode === "preview" &&
    acknowledged &&
    permissionReady &&
    remaining !== null &&
    remaining > 0 &&
    pending === 0 &&
    hasWork;
  return {
    beat,
    fresh,
    acknowledged,
    used,
    pending,
    remaining,
    expired,
    label,
    explanation,
    canStart,
    permissionReady,
  };
}

/** Capture roots are accepted issue history, distinct from assignment/reminder receipts. */
export function automationActivity(context: DispatchContext) {
  const signers = new Set(
    activeRecords(context.records, "automation").map(
      (record) => record.record.payload.controls.service_pubkey,
    ),
  );
  const entries = (context.automation?.actions ?? [])
    .filter((action) => signers.has(action.signer))
    .map((action) => ({
      eventId: action.eventId,
      issueId: action.issueId,
      createdAt: action.createdAt,
      title:
        context.issues.find((issue) => issue.issueId === action.issueId)
          ?.title ?? "Issue no longer available",
      result:
        action.action === "assign"
          ? "Sent to an agent"
          : action.action === "escalate"
            ? "Reminder sent"
            : "Work updated",
      detail: action.result,
    }));
  for (const issue of context.issues) {
    const root = context.activity.find(
      (event) =>
        event.eventId === issue.rootEvent &&
        event.issueId === issue.issueId &&
        event.revision === 0 &&
        !event.import &&
        signers.has(event.authorPubkey),
    );
    if (root && issue.sourceEventId)
      entries.push({
        eventId: root.eventId,
        issueId: issue.issueId,
        createdAt: root.createdAt,
        title: issue.title,
        result: "Created from a message",
        detail: "The original message is linked on the issue.",
      });
  }
  return [...new Map(entries.map((entry) => [entry.eventId, entry])).values()]
    .sort(
      (a, b) => b.createdAt - a.createdAt || a.eventId.localeCompare(b.eventId),
    )
    .slice(0, 20);
}
