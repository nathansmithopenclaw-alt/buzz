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
  | "transcribing";

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
  /** Cancels and also drops any result still waiting to be delivered. */
  discard: () => void;
};

const targets = new Map<number, DictationTarget>();
let nextTargetId = 1;
let recorderOwner: number | null = null;
/**
 * Settles once every backend release requested so far has landed. The
 * backend allows one session at a time, so a new start waits on this rather
 * than racing a stale session's cancel for the slot.
 */
let backendReleased: Promise<unknown> = Promise.resolve();

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

/** What the hotkey machine asks for (see `dictationHotkey.ts`). */
export type HotkeyRoute = "start-hold" | "stop-hold" | "toggle" | "cancel";

/**
 * Routes one hotkey action to the right composer. A typing chord's `cancel`
 * only ever stops a capture in progress: it must never abort another
 * composer's in-flight transcription or cleanup.
 */
export function routeHotkeyAction(action: HotkeyRoute): void {
  if (action === "cancel") {
    recordingDictationTarget()?.cancel();
    return;
  }
  const target = pickDictationTarget();
  if (!target) return;
  if (action === "start-hold") target.start("hold");
  else if (action === "stop-hold") target.stop();
  else if (["starting", "recording"].includes(target.status())) target.stop();
  else target.start("toggle");
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

/**
 * Records backend work that frees the session slot (a cancel, or a start
 * that will cancel itself once it lands stale). Never rejects.
 */
export function trackBackendRelease(work: Promise<unknown>): void {
  const settled = work.catch(() => {});
  backendReleased = Promise.all([backendReleased, settled]);
}

/** Resolves once every tracked backend release has settled. */
export function backendIdle(): Promise<unknown> {
  return backendReleased;
}

/** Cancels every dictation and clears the recorder slot. */
export function resetDictationState(): void {
  for (const target of targets.values()) target.discard();
  recorderOwner = null;
}
