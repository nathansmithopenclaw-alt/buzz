import {
  scheduleFormFromTrigger,
  SCHEDULE_FREQUENCY_LABELS,
} from "@/features/workflows/ui/workflowSchedule";
import type { NamedSummary } from "./namedSummaries";
export function summaryScheduleLabel(config: NamedSummary): string {
  if (config.archived) return "Archived";
  if (config.paused) return "Paused";
  if (!config.schedule) return "Manual only";
  if (config.schedule.dailyTimes)
    return `Daily at ${config.schedule.dailyTimes.join(", ")} · ${config.timezone}`;
  const form = scheduleFormFromTrigger({ on: "schedule", ...config.schedule });
  const label =
    form.frequency === "custom_interval"
      ? `Every ${form.customInterval}`
      : SCHEDULE_FREQUENCY_LABELS[form.frequency];
  return `${label}${["daily", "weekly", "monthly"].includes(form.frequency) ? ` at ${form.time}` : ""} · ${config.timezone}`;
}
