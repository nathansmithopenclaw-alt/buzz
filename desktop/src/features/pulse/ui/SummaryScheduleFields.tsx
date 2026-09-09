import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
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
          {schedule.dailyTimes ? (
            <div className="space-y-3">
              {schedule.dailyTimes.map((time, index) => (
                // Each slot is a controlled time input; its position remains stable while editing.
                // biome-ignore lint/suspicious/noArrayIndexKey: Preserve focus while changing the time value.
                <div key={index} className="flex items-center gap-2">
                  <Input
                    type="time"
                    aria-label={`Daily time ${index + 1}`}
                    value={time}
                    disabled={disabled}
                    onChange={(e) =>
                      onChange({
                        on: "schedule",
                        dailyTimes: (schedule.dailyTimes ?? []).map((t, i) =>
                          i === index ? e.target.value : t,
                        ),
                      })
                    }
                  />
                  <Button
                    variant="ghost"
                    disabled={
                      disabled || (schedule.dailyTimes ?? []).length === 1
                    }
                    onClick={() =>
                      onChange({
                        on: "schedule",
                        dailyTimes: (schedule.dailyTimes ?? []).filter(
                          (_, i) => i !== index,
                        ),
                      })
                    }
                  >
                    Remove
                  </Button>
                </div>
              ))}
              <Button
                variant="outline"
                disabled={disabled || schedule.dailyTimes.length >= 24}
                onClick={() =>
                  onChange({
                    on: "schedule",
                    dailyTimes: [...(schedule.dailyTimes ?? []), "17:00"],
                  })
                }
              >
                Add time
              </Button>
              <Button
                variant="ghost"
                disabled={disabled}
                onClick={() => onChange({ on: "schedule", cron: "0 9 * * *" })}
              >
                Other frequencies
              </Button>
            </div>
          ) : (
            <>
              <Button
                variant="outline"
                disabled={disabled}
                onClick={() =>
                  onChange({ on: "schedule", dailyTimes: ["09:00"] })
                }
              >
                Set daily times
              </Button>
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
            </>
          )}
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
