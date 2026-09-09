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
  const days = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
  ];
  const detail =
    form.frequency === "weekly"
      ? ` on ${form.weekday
          .split(",")
          .map((day) => days[Number(day)] ?? day)
          .join(", ")}`
      : form.frequency === "monthly"
        ? ` on day ${form.monthDay}`
        : form.frequency === "custom_cron"
          ? ` (${form.customCron})`
          : "";
  return `${label}${detail}${["daily", "weekly", "monthly"].includes(form.frequency) ? ` at ${form.time}` : ""} · ${config.timezone}`;
}
