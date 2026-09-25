import * as React from "react";
import { toast } from "sonner";
import { AgentDropdownSelect } from "@/features/agents/ui/agentConfigControls";
import { SettingsOptionRow } from "@/features/settings/ui/SettingsOptionGroup";
import { Button } from "@/shared/ui/button";
import {
  DICTATION_DEVICE_STORAGE_KEY,
  listAudioInputs,
  revealAudioInputs,
} from "../lib/audioCapture";

function readDevice(): string {
  try {
    return window.localStorage.getItem(DICTATION_DEVICE_STORAGE_KEY) ?? "";
  } catch {
    return "";
  }
}

/**
 * The dictation microphone: a per-device convenience kept in local storage
 * ("" = system default). The list refreshes when devices come and go.
 */
function useMicrophoneChoice(enabled: boolean) {
  const [inputs, setInputs] = React.useState<
    { deviceId: string; label: string }[]
  >([]);
  const [deviceId, setDeviceId] = React.useState(readDevice);
  const refresh = React.useCallback(
    () =>
      listAudioInputs()
        .then(setInputs)
        .catch(() => setInputs([])),
    [],
  );
  React.useEffect(() => {
    if (!enabled) return;
    const onChange = () => void refresh();
    onChange();
    navigator.mediaDevices?.addEventListener?.("devicechange", onChange);
    return () =>
      navigator.mediaDevices?.removeEventListener?.("devicechange", onChange);
  }, [enabled, refresh]);
  // Until Buzz has used the mic this session, WebKit hides device names and
  // ids; opening the default mic for a moment reveals them.
  const reveal = () =>
    void revealAudioInputs()
      .then(refresh)
      .catch(() =>
        toast.error("Allow Buzz to use your microphone to list microphones."),
      );
  const choose = (next: string) => {
    setDeviceId(next);
    try {
      if (next) window.localStorage.setItem(DICTATION_DEVICE_STORAGE_KEY, next);
      else window.localStorage.removeItem(DICTATION_DEVICE_STORAGE_KEY);
    } catch {
      toast.error("Buzz could not save your microphone choice.");
    }
  };
  return { inputs, deviceId, choose, reveal };
}

export function DictationMicrophoneRow() {
  const microphone = useMicrophoneChoice(true);
  return (
    <SettingsOptionRow>
      <label className="font-medium" htmlFor="dictation-microphone">
        Microphone
      </label>
      <div className="flex w-80 max-w-full items-center gap-2">
        {microphone.inputs.length === 0 ? (
          <Button
            onClick={microphone.reveal}
            size="sm"
            type="button"
            variant="outline"
          >
            Find microphones
          </Button>
        ) : null}
        <AgentDropdownSelect
          id="dictation-microphone"
          onValueChange={microphone.choose}
          value={microphone.deviceId}
          options={[
            { value: "", label: "System default" },
            ...(microphone.deviceId &&
            !microphone.inputs.some(
              (input) => input.deviceId === microphone.deviceId,
            )
              ? [
                  {
                    value: microphone.deviceId,
                    label: "Saved microphone (unavailable)",
                  },
                ]
              : []),
            ...microphone.inputs.map((input) => ({
              value: input.deviceId,
              label: input.label,
            })),
          ]}
        />
      </div>
    </SettingsOptionRow>
  );
}
