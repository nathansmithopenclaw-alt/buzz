import type { DispatchContext } from "@/features/dispatch/types";
import type { Notice } from "@/features/dispatch/conversationUpdates";
import type { RelayEvent } from "@/shared/api/types";
const hex = (bytes: Uint8Array) =>
  [...bytes].map((value) => value.toString(16).padStart(2, "0")).join("");
export async function mockCaptureRequestId(source: string) {
  const namespace = Uint8Array.from(
    "0073632d64697370617463682d76310000".match(/../g) ?? [],
    (value) => Number.parseInt(value, 16),
  );
  const input = new Uint8Array([
    ...namespace,
    ...new TextEncoder().encode(`pin:${source}`),
  ]);
  const bytes = new Uint8Array(
    await crypto.subtle.digest("SHA-1", input),
  ).slice(0, 16);
  bytes[6] = (bytes[6] & 15) | 80;
  bytes[8] = (bytes[8] & 63) | 128;
  const value = hex(bytes);
  return `${value.slice(0, 8)}-${value.slice(8, 12)}-${value.slice(12, 16)}-${value.slice(16, 20)}-${value.slice(20)}`;
}
async function noticeKey(channel: string, notice: Notice) {
  return hex(
    new Uint8Array(
      await crypto.subtle.digest(
        "SHA-256",
        new TextEncoder().encode(
          `dispatch-notice-v1:${notice.workspace}:${notice.issue}:${channel}:${notice.source_event ?? ""}:${notice.change_event}:${notice.purpose}`,
        ),
      ),
    ),
  );
}
/** Renderer-only fixtures. Signature/authority proofs run against the Rust SDK and real commands. */
export async function dispatchMockCommand(
  command: string,
  args: Record<string, unknown>,
  context: DispatchContext | null,
) {
  if (command === "dispatch_capture_request_id")
    return mockCaptureRequestId(String(args.sourceEvent));
  if (command === "validate_dispatch_notice_readback") {
    const event = args.event as RelayEvent,
      notice = args.notice as Notice;
    if (
      event.tags.find((tag) => tag[0] === "d")?.[1] !==
      (await noticeKey(String(args.channelId), notice))
    )
      throw new Error("Notice readback mismatch");
    return event;
  }
  if (command === "build_dispatch_notice_event") {
    const notice = args.notice as Notice,
      source = args.source as RelayEvent | null;
    const tags = [
      ["h", String(args.channelId)],
      ["t", "dispatch-update"],
      ["d", await noticeKey(String(args.channelId), notice)],
      [
        "dispatch",
        notice.workspace,
        notice.issue,
        notice.change_event,
        notice.purpose,
      ],
    ];
    if (source) tags.push(["q", source.id], ["e", source.id, "", "root"]);
    for (const person of (args.mentions ?? []) as string[])
      tags.push(["p", person]);
    return {
      kind: 9,
      content: args.body,
      tags,
      createdAt: Math.floor(Date.now() / 1000),
    };
  }
  if (command === "build_dispatch_task_review_event") {
    const input = args.input as {
      task_id: string;
      expected_event: string;
      accepted: boolean;
      evidence: string;
      notes: string;
    };
    const task = context?.linkedTasks.find(
      (task) => task.taskId === input.task_id,
    );
    if (
      task?.status !== "done" ||
      task.eventId !== input.expected_event ||
      task.assignee === args.authorPubkey ||
      ![task.reviewer, task.assigner].includes(String(args.authorPubkey))
    )
      throw new Error("This result cannot be reviewed by the current person");
    const status = input.accepted ? "verified" : "failed";
    const envelope = {
      v: 3,
      task_id: task.taskId,
      title: task.title,
      status,
      assignee: task.assignee,
      assigner: task.assigner,
      authority: {
        assignment_event_id: task.assignmentEventId,
        previous_event_id: task.eventId,
        reviewer: task.reviewer,
      },
      deliverables: task.deliverables ?? [],
      verification: {
        method: task.verification?.method ?? "Independent review",
        evidence_ref: input.evidence,
      },
      blocked_on: null,
      notes: input.notes,
      tracker_issue_id: task.issueId,
      dispatch_request_id: task.dispatchRequestId,
    };
    return {
      kind: 9,
      createdAt: Math.floor(Date.now() / 1000),
      content: `Review result\n\n\`\`\`buzz-task\n${JSON.stringify(envelope)}\n\`\`\``,
      tags: [
        ["h", task.channelId],
        ["t", "buzz-task"],
        ["task", task.taskId, status],
        ["e", task.assignmentEventId ?? task.eventId, "", "root"],
        ["e", task.eventId, "", "reply"],
      ],
    };
  }
  throw new Error(`Unknown Dispatch fixture command: ${command}`);
}
