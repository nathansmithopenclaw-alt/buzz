import { AudioLines, LoaderCircle } from "lucide-react";
import * as React from "react";

import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { DictationStatus } from "../lib/dictationSession";

/** Human label for a `KeyboardEvent.code` shortcut. */
export function hotkeyLabel(code: string, isMac: boolean): string {
  if (code === "AltRight") return isMac ? "Right Option" : "Right Alt";
  if (code === "AltLeft") return isMac ? "Left Option" : "Left Alt";
  if (code === "MetaRight") return isMac ? "Right Command" : "Right Windows";
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  return code;
}

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = String(seconds % 60).padStart(2, "0");
  return `${m}:${s}`;
}

const BAR_KEYS = ["bar-0", "bar-1", "bar-2", "bar-3", "bar-4"] as const;

const STATUS_ANNOUNCEMENT: Record<DictationStatus, string> = {
  idle: "",
  starting: "Starting dictation",
  recording: "Dictation recording",
  transcribing: "Transcribing dictation",
  cleaning: "Cleaning up dictation",
};

export type DictationButtonProps = {
  status: DictationStatus;
  /** Latest input level (0..1); polled by the button while recording. */
  getLevel: () => number;
  /** Epoch ms when recording started. */
  startedAt: number;
  onToggle: () => void;
  onCaptureSelection?: () => void;
  /** Shown instead of the action when dictation can't run right now. */
  disabledReason: string | null;
  shortcut: string;
};

/**
 * Composer dictation control (sound-wave icon). Idle: starts dictation.
 * Recording: shows a live level waveform and timer; clicking stops. While
 * transcribing or cleaning up it shows progress and cannot be pressed.
 */
/** Waveform + clock, local to the button so the composer never re-renders per tick. */
function useRecordingMeter(
  active: boolean,
  getLevel: () => number,
  startedAt: number,
) {
  const [levels, setLevels] = React.useState<number[]>(() =>
    BAR_KEYS.map(() => 0),
  );
  const [elapsedSeconds, setElapsedSeconds] = React.useState(0);
  React.useEffect(() => {
    if (!active) {
      setLevels(BAR_KEYS.map(() => 0));
      setElapsedSeconds(0);
      return;
    }
    const timer = window.setInterval(() => {
      setLevels((previous) => [...previous.slice(1), getLevel()]);
      setElapsedSeconds(
        Math.max(0, Math.floor((Date.now() - startedAt) / 1000)),
      );
    }, 100);
    return () => window.clearInterval(timer);
  }, [active, getLevel, startedAt]);
  return { levels, elapsedSeconds };
}

export function DictationButton({
  status,
  getLevel,
  startedAt,
  onToggle,
  onCaptureSelection,
  disabledReason,
  shortcut,
}: DictationButtonProps) {
  const recording = status === "recording" || status === "starting";
  const { levels, elapsedSeconds } = useRecordingMeter(
    status === "recording",
    getLevel,
    startedAt,
  );
  const processing = status === "transcribing" || status === "cleaning";
  const label = recording
    ? "Stop dictation"
    : processing
      ? status === "transcribing"
        ? "Transcribing dictation"
        : "Cleaning up dictation"
      : "Dictate";
  const tooltip =
    disabledReason ??
    (recording
      ? `Stop dictation (${shortcut})`
      : `Dictate (tap or hold ${shortcut})`);
  return (
    <>
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <Button
            aria-label={label}
            aria-pressed={recording}
            className={recording ? "gap-1.5 px-2 text-primary" : undefined}
            data-testid="composer-dictation-button"
            disabled={Boolean(disabledReason) || processing}
            onClick={onToggle}
            onMouseDown={onCaptureSelection}
            size={recording ? "sm" : "icon"}
            type="button"
            variant="ghost"
          >
            {processing ? (
              <LoaderCircle aria-hidden className="animate-spin" />
            ) : recording ? (
              <span
                aria-hidden
                className="inline-flex h-4 items-center gap-0.5"
              >
                {BAR_KEYS.map((key, index) => (
                  <span
                    key={key}
                    className="w-0.5 rounded-full bg-current transition-[height] duration-100"
                    style={{
                      height: `${Math.max(0.2, Math.min(1, levels[index] ?? 0))}rem`,
                    }}
                  />
                ))}
              </span>
            ) : (
              <span className="inline-flex">
                <AudioLines aria-hidden />
              </span>
            )}
            {recording ? (
              <span className="text-xs tabular-nums">
                {formatElapsed(elapsedSeconds)}
              </span>
            ) : null}
          </Button>
        </TooltipTrigger>
        <TooltipContent>{tooltip}</TooltipContent>
      </Tooltip>
      <span className="sr-only" role="status" aria-live="polite">
        {STATUS_ANNOUNCEMENT[status]}
      </span>
    </>
  );
}
