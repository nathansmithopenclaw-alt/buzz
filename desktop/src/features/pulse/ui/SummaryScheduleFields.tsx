import * as React from "react";
import { WorkflowScheduleFields } from "@/features/workflows/ui/WorkflowScheduleFields";
import {
  summaryScheduleError,
  type SummarySchedule,
} from "../lib/summarySchedule";
import { Switch } from "@/shared/ui/switch";

export function SummaryScheduleFields({
  schedule,
  timezone,
  disabled,
  onChange,
}: {
  schedule: SummarySchedule | null;
  timezone: string;
  disabled?: boolean;
  onChange: (schedule: SummarySchedule | null) => void;
}) {
  const previous = React.useRef<SummarySchedule>({
    on: "schedule",
    cron: "0 9 * * *",
  });
  React.useEffect(() => {
    if (schedule) previous.current = schedule;
  }, [schedule]);
  const error = summaryScheduleError(schedule);
  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <p className="text-sm font-medium" id="summary-scheduled-label">
            Run on a schedule
          </p>
          <p className="text-xs text-muted-foreground">
            You can also run a Summary manually at any time.
          </p>
        </div>
        <Switch
          aria-labelledby="summary-scheduled-label"
          checked={schedule !== null}
          disabled={disabled}
          onCheckedChange={(enabled) =>
            onChange(enabled ? previous.current : null)
          }
        />
      </div>
      {schedule ? (
        <>
          <p className="text-sm text-muted-foreground">
            Times use {timezone}, including daylight saving changes.
          </p>
          <WorkflowScheduleFields
            disabled={disabled}
            timezone={timezone}
            trigger={schedule}
            onUpdate={(trigger) =>
              onChange({
                on: "schedule",
                cron: trigger.cron,
                interval: trigger.interval,
              })
            }
          />
          <p className="text-xs text-muted-foreground">
            For several runs a day, Custom cron accepts lists of hours. For
            example, 0 9,13,17 * * * runs at 09:00, 13:00 and 17:00.
          </p>
        </>
      ) : (
        <p className="text-sm text-muted-foreground">
          Manual only. No scheduled runs.
        </p>
      )}
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
}
