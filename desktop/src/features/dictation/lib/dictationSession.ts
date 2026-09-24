/**
 * Window-level dictation bookkeeping: which composers can receive dictation,
 * which one the hotkey should drive, and the single recorder slot (the
 * backend additionally allows one session across all windows).
 *
 * Community-scoped: `resetDictationState` cancels everything on a community
 * switch (registered in `resetCommunityState`).
 */

export type DictationStatus =
  | "idle"
  | "starting"
  | "recording"
  | "transcribing"
  | "cleaning";

export type DictationTarget = {
  /** The channel's main composer (fallback target for the hotkey). */
  isMain: boolean;
  isFocused: () => boolean;
  /** Last time this composer's editor had focus (0 = never). */
  focusedAt: () => number;
  status: () => DictationStatus;
  /** Starts dictation (`hold`: push-to-talk; `toggle`: hands-free). */
  start: (mode: "hold" | "toggle") => void;
  stop: () => void;
  cancel: () => void;
};

const targets = new Map<number, DictationTarget>();
let nextTargetId = 1;
let recorderOwner: number | null = null;

/** Registers a composer as a dictation target; returns its unregister. */
export function registerDictationTarget(target: DictationTarget): {
  id: number;
  unregister: () => void;
} {
  const id = nextTargetId++;
  targets.set(id, target);
  return {
    id,
    unregister: () => {
      targets.delete(id);
      if (recorderOwner === id) recorderOwner = null;
    },
  };
}

/**
 * The composer the hotkey drives: the one already recording, else the
 * focused one, else the most recently focused, else a main composer.
 */
export function pickDictationTarget(): DictationTarget | undefined {
  const all = [...targets.values()];
  const active = all.find((t) =>
    ["starting", "recording"].includes(t.status()),
  );
  if (active) return active;
  const focused = all.find((t) => t.isFocused());
  if (focused) return focused;
  const recent = [...all].sort((a, b) => b.focusedAt() - a.focusedAt())[0];
  if (recent && recent.focusedAt() > 0) return recent;
  return all.find((t) => t.isMain) ?? all[0];
}

/** Any composer currently capturing audio. */
export function recordingDictationTarget(): DictationTarget | undefined {
  return [...targets.values()].find((t) =>
    ["starting", "recording"].includes(t.status()),
  );
}

/** Claims the window's single recorder for target `id`. */
export function claimRecorder(id: number): boolean {
  if (recorderOwner !== null && recorderOwner !== id) return false;
  recorderOwner = id;
  return true;
}

export function releaseRecorder(id: number): void {
  if (recorderOwner === id) recorderOwner = null;
}

/** Cancels every dictation and clears the recorder slot. */
export function resetDictationState(): void {
  for (const target of targets.values()) target.cancel();
  recorderOwner = null;
}
