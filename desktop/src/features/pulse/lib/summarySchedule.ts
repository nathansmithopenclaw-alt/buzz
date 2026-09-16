import { dailyTimesToCrons } from "./summaryDailyTimes";
import {
  createWorkflow,
  deleteWorkflow,
  getChannelWorkflows,
  getWorkflow,
  updateWorkflow,
} from "@/shared/api/tauriWorkflows";
import type { Workflow } from "@/shared/api/workflowTypes";

import { cronExpressionError } from "@/features/workflows/ui/cronExpression";
import { parseDurationSeconds } from "@/features/workflows/ui/workflowDuration";
import type { TriggerConfig } from "@/features/workflows/ui/workflowFormTypes";

export type SummarySchedule = Pick<TriggerConfig, "cron" | "interval"> & {
  on: "schedule";
  dailyTimes?: string[];
};

export function summaryScheduleError(
  schedule: SummarySchedule | null,
): string | null {
  if (!schedule) return null;
  if (schedule.dailyTimes) {
    try {
      dailyTimesToCrons(schedule.dailyTimes);
      return null;
    } catch (error) {
      return error instanceof Error ? error.message : "Invalid daily times";
    }
  }
  if (
    schedule.on !== "schedule" ||
    Boolean(schedule.cron) === Boolean(schedule.interval)
  ) {
    return "Choose a schedule with either a run time or an interval.";
  }
  if (schedule.cron) return cronExpressionError(schedule.cron);
  const seconds = parseDurationSeconds(schedule.interval ?? "");
  return seconds !== null && seconds >= 60
    ? null
    : "Choose an interval of at least one minute.";
}

export function summaryPrompt(
  ownerPubkey: string,
  reporterName: string,
): string {
  const name = sanitizeMentionName(reporterName) || "Nectar";
  return `@${name} Write a Summary of new activity since your previous Summary. Read the owner's configured conversations with buzz briefs config get --owner ${ownerPubkey}, then buzz briefs read --owner ${ownerPubkey} --channel <id> --since <watermark>. Page with --until and --before-id when needed. Publish a kind 40030 report for that owner from your watermarks. Do not invent work. If there is no new activity, report that without republishing previous material. Advance watermarks only for conversations actually read.`;
}

export const SUMMARY_WORKFLOW_NAME = "Summary";

export function summaryWorkflowYaml(input: {
  hour: number;
  minute: number;
  timezone: string;
  reporterName: string;
  ownerPubkey?: string;
  schedule?: SummarySchedule;
}): string {
  const hour = clampInt(input.hour, 0, 23);
  const minute = clampInt(input.minute, 0, 59);
  const timezone = sanitizeYamlScalar(input.timezone) || "UTC";
  const schedule = input.schedule ?? {
    on: "schedule",
    cron: `${minute} ${hour} * * *`,
  };
  const error = summaryScheduleError(schedule);
  if (error) throw new Error(error);
  return [
    `name: ${SUMMARY_WORKFLOW_NAME}`,
    "trigger:",
    "  on: schedule",
    schedule.cron
      ? `  cron: '${schedule.cron.trim()}'`
      : `  interval: ${JSON.stringify(schedule.interval)}`,
    `  timezone: ${timezone}`,
    "steps:",
    "  - id: run",
    "    action: send_message",
    `    text: ${JSON.stringify(summaryPrompt(input.ownerPubkey ?? "<owner>", input.reporterName))}`,
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
  schedule?: SummarySchedule | null;
}): Promise<string | null> {
  if (!input.hostChannelId || input.schedule === null) {
    await deleteSummaryWorkflow(input.previousWorkflowId);
    return null;
  }

  const yaml = summaryWorkflowYaml({
    ...input,
    schedule: input.schedule ?? undefined,
  });
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
