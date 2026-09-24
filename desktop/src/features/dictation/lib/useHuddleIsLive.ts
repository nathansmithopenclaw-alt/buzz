import * as React from "react";
import { listen } from "@tauri-apps/api/event";

import { invokeTauri } from "@/shared/api/tauri";

type HuddleStateSnapshot = { phase?: string };

/**
 * `true` while a huddle holds (or is acquiring) the microphone, from the
 * backend's authoritative huddle phase. Safe outside the huddle provider;
 * defaults to `false` where huddles are unavailable.
 */
export function useHuddleIsLive(): boolean {
  const [live, setLive] = React.useState(false);
  React.useEffect(() => {
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
  }, []);
  return live;
}
