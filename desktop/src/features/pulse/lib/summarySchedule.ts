import {
  createWorkflow,
  deleteWorkflow,
  getChannelWorkflows,
  getWorkflow,
  updateWorkflow,
} from "@/shared/api/tauriWorkflows";
import type { Workflow } from "@/shared/api/workflowTypes";

export const SUMMARY_WORKFLOW_NAME = "Summary";

export function summaryWorkflowYaml(input: {
  hour: number;
  minute: number;
  timezone: string;
  reporterName: string;
  ownerPubkey?: string;
}): string {
  const hour = clampInt(input.hour, 0, 23);
  const minute = clampInt(input.minute, 0, 59);
  const timezone = sanitizeYamlScalar(input.timezone) || "UTC";
  const reporterName = sanitizeMentionName(input.reporterName) || "Nectar";
  return [
    `name: ${SUMMARY_WORKFLOW_NAME}`,
    "trigger:",
    "  on: schedule",
    `  cron: '${minute} ${hour} * * *'`,
    `  timezone: ${timezone}`,
    "steps:",
    "  - id: run",
    "    action: send_message",
    `    text: ${JSON.stringify(`@${reporterName} Write today's Summary. Read the owner's configured conversations with buzz briefs config get --owner ${input.ownerPubkey ?? "<owner>"}, then buzz briefs read --owner ${input.ownerPubkey ?? "<owner>"} --channel <id> --since <watermark>. Page with --until and --before-id when needed. Publish a kind 40030 report for that owner from your watermarks. Do not invent work.`)}`,
    "",
  ].join("\n");
}

export function findSummaryWorkflow(
  workflows: readonly Workflow[],
  ownerPubkey: string,
): Workflow | undefined {
  const owner = ownerPubkey.toLowerCase();
  return workflows.find(
    (workflow) =>
      workflow.name === SUMMARY_WORKFLOW_NAME &&
      workflow.ownerPubkey.toLowerCase() === owner,
  );
}

export async function syncSummarySchedule(input: {
  ownerPubkey: string;
  previousWorkflowId: string | null;
  hostChannelId: string | null;
  hour: number;
  minute: number;
  timezone: string;
  reporterName: string;
}): Promise<string | null> {
  const yaml = summaryWorkflowYaml(input);
  if (!input.hostChannelId) {
    await deleteSummaryWorkflow(input.previousWorkflowId);
    return null;
  }

  const existing = await loadSummaryWorkflow(
    input.previousWorkflowId,
    input.hostChannelId,
    input.ownerPubkey,
  );
  if (existing && existing.channelId === input.hostChannelId) {
    const updated = await updateWorkflow(existing.id, yaml, existing.revision);
    return updated.workflow.id;
  }
  if (existing) {
    await deleteSummaryWorkflow(existing.id);
  }
  const created = await createWorkflow(input.hostChannelId, yaml);
  return created.workflow.id;
}

async function loadSummaryWorkflow(
  previousWorkflowId: string | null,
  hostChannelId: string,
  ownerPubkey: string,
): Promise<Workflow | null> {
  if (previousWorkflowId) {
    try {
      return await getWorkflow(previousWorkflowId);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message !== "workflow not found") throw error;
      // An explicitly missing workflow can be recreated; transport errors cannot.
    }
  }
  const workflows = await getChannelWorkflows(hostChannelId);
  return findSummaryWorkflow(workflows, ownerPubkey) ?? null;
}

async function deleteSummaryWorkflow(workflowId: string | null): Promise<void> {
  if (!workflowId) return;
  await deleteWorkflow(workflowId);
}

function clampInt(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, Math.trunc(value)));
}

function sanitizeYamlScalar(value: string): string {
  return value.trim().replace(/[^\w/+-]/g, "");
}

function sanitizeMentionName(value: string): string {
  return value
    .trim()
    .replace(/[\n\r@]/g, "")
    .slice(0, 64);
}
