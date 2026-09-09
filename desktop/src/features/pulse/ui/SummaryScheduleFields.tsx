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
  onTimezoneChange,
  disabled,
  onChange,
}: {
  schedule: SummarySchedule | null;
  timezone: string;
  onTimezoneChange?: (timezone: string) => void;
  disabled?: boolean;
  onChange: (schedule: SummarySchedule | null) => void;
}) {
  const timezoneId = React.useId();
  const dailyDraft = React.useRef<SummarySchedule>({
    on: "schedule",
    dailyTimes: ["09:00"],
  });
  const frequencyDraft = React.useRef<SummarySchedule>({
    on: "schedule",
    cron: "0 9 * * *",
  });
  const previous = React.useRef<SummarySchedule>({
    on: "schedule",
    cron: "0 9 * * *",
  });
  React.useEffect(() => {
    if (schedule) {
      previous.current = schedule;
      if (schedule.dailyTimes) dailyDraft.current = schedule;
      else frequencyDraft.current = schedule;
    }
  }, [schedule]);
  const timezoneError = (() => {
    try {
      new Intl.DateTimeFormat(undefined, { timeZone: timezone });
      return null;
    } catch {
      return "Choose a valid timezone, such as America/New_York.";
    }
  })();
  const error = timezoneError ?? summaryScheduleError(schedule);
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
          {onTimezoneChange ? (
            <label className="block space-y-2 text-sm" htmlFor={timezoneId}>
              Timezone
              <Input
                id={timezoneId}
                list={`${timezoneId}-options`}
                value={timezone}
                disabled={disabled}
                onChange={(e) => onTimezoneChange(e.target.value)}
              />
              <datalist id={`${timezoneId}-options`}>
                {["UTC", ...Intl.supportedValuesOf("timeZone")].map((zone) => (
                  <option key={zone} value={zone} />
                ))}
              </datalist>
            </label>
          ) : null}
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
                onClick={() => onChange(frequencyDraft.current)}
              >
                Other frequencies
              </Button>
            </div>
          ) : (
            <>
              <Button
                variant="outline"
                disabled={disabled}
                onClick={() => onChange(dailyDraft.current)}
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
