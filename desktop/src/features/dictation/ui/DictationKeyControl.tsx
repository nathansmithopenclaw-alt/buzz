import * as React from "react";
import { Button } from "@/shared/ui/button";
import { isMacPlatform } from "@/shared/lib/platform";
import {
  claimDictationKey,
  isAltGraphEvent,
  isAvailableDictationKey,
} from "../hotkeyCapture";
import { hotkeyLabel } from "./DictationButton";

export function DictationKeyControl({
  value,
  disabled,
  onChange,
}: {
  value: string;
  disabled: boolean;
  onChange: (key: string) => void;
}) {
  const onChangeRef = React.useRef(onChange);
  onChangeRef.current = onChange;
  const [capturing, setCapturing] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (!capturing) return;
    const release = claimDictationKey();
    let candidate: string | null = null;
    const cancel = () => setCapturing(false);
    const down = (event: KeyboardEvent) => {
      if (event.isComposing || isAltGraphEvent(event, isMacPlatform())) {
        candidate = null;
        return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
      if (event.code === "Escape") {
        cancel();
        return;
      }
      if (event.repeat) return;
      if (
        candidate ||
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey ||
        (event.altKey && !event.code.startsWith("Alt"))
      ) {
        candidate = null;
        setError("Choose a single key");
      } else if (!isAvailableDictationKey(event.code)) {
        candidate = null;
        setError("This key conflicts with typing or another shortcut");
      } else {
        candidate = event.code;
        setError(null);
      }
    };
    const up = (event: KeyboardEvent) => {
      if (isAltGraphEvent(event, isMacPlatform())) {
        candidate = null;
        return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
      if (candidate !== event.code) return;
      onChangeRef.current(candidate);
      setCapturing(false);
    };
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    window.addEventListener("blur", cancel);
    return () => {
      release();
      window.removeEventListener("keydown", down, true);
      window.removeEventListener("keyup", up, true);
      window.removeEventListener("blur", cancel);
    };
  }, [capturing]);
  return (
    <div className="space-y-1 text-right">
      <Button
        aria-label="Change dictation key"
        disabled={disabled}
        onClick={() => {
          setError(null);
          setCapturing((active) => !active);
        }}
        variant="outline"
      >
        {capturing ? "Press a key…" : hotkeyLabel(value, isMacPlatform())}
      </Button>
      {capturing ? (
        <p className="text-xs text-muted-foreground">Esc to cancel</p>
      ) : null}
      {error ? (
        <p className="text-xs text-destructive" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
