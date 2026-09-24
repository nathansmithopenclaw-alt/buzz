import * as React from "react";
import { listen } from "@tauri-apps/api/event";

import { invokeTauri } from "@/shared/api/tauri";

type HuddleStateSnapshot = { phase?: string };

/**
 * `true` while a huddle holds (or is acquiring) the microphone, from the
 * backend's authoritative huddle phase. Safe outside the huddle provider;
 * defaults to `false` where huddles are unavailable. Inert (no invoke, no
 * listener) while `enabled` is false, so composers pay nothing when the
 * dictation experiment is off.
 */
export function useHuddleIsLive(enabled: boolean): boolean {
  const [live, setLive] = React.useState(false);
  React.useEffect(() => {
    if (!enabled) {
      setLive(false);
      return;
    }
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    const apply = (state: HuddleStateSnapshot | null | undefined) => {
      if (!cancelled) setLive(Boolean(state?.phase && state.phase !== "idle"));
    };
    invokeTauri<HuddleStateSnapshot>("get_huddle_state")
      .then(apply)
      .catch(() => apply(null));
    listen<HuddleStateSnapshot>("huddle-state-changed", (event) =>
      apply(event.payload),
    )
      .then((cleanup) => {
        if (cancelled) cleanup();
        else unlisten = cleanup;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [enabled]);
  return live;
}
