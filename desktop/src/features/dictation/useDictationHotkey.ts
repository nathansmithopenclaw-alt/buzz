import * as React from "react";

import { DictationHotkeyMachine } from "./lib/dictationHotkey";
import {
  recordingDictationTarget,
  routeHotkeyAction,
} from "./lib/dictationSession";
import { useDictationSettings } from "./useDictationSettings";

const DEFAULT_HOTKEY = "AltRight";

/**
 * Window-level dictation shortcut. Mounted once in the app shell; routes the
 * hold/tap machine to the composer that should receive dictation, and lets
 * Esc cancel an active recording before other Esc handlers run.
 */
export function useDictationHotkey(enabled: boolean) {
  const { settings } = useDictationSettings(enabled);
  const hotkeyRef = React.useRef(DEFAULT_HOTKEY);
  hotkeyRef.current = settings?.hotkey ?? DEFAULT_HOTKEY;

  React.useEffect(() => {
    if (!enabled) return;
    const machine = new DictationHotkeyMachine(
      () => hotkeyRef.current,
      routeHotkeyAction,
      {
        now: () => performance.now(),
        set: (fn, ms) => window.setTimeout(fn, ms),
        clear: (handle) => window.clearTimeout(handle as number),
      },
    );
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) {
        const recording = recordingDictationTarget();
        if (recording) {
          event.preventDefault();
          event.stopPropagation();
          recording.cancel();
          return;
        }
      }
      if (machine.keydown(event)) event.preventDefault();
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (machine.keyup(event)) event.preventDefault();
    };
    const onBlur = () => machine.blur();
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", onBlur);
      machine.blur();
    };
  }, [enabled]);
}
