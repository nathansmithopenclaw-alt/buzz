/**
 * Dictation hotkey (default Right Option on macOS / Right Alt elsewhere).
 *
 * - Quick tap (< 300 ms, no other key): toggle hands-free dictation.
 * - Hold ≥ 300 ms: push-to-talk; release stops.
 * - The mic never opens before 300 ms of a solo hold, so Option/AltGr typing
 *   (é, ü, and AltGr+Q = "@" on many keyboards) never flashes the mic; any
 *   other key during the hold cancels and passes through untouched.
 * - AltGr (Windows reports Ctrl+AltRight) is ignored entirely.
 */

export const HOLD_THRESHOLD_MS = 300;

/** What the machine asks its host to do. */
export type HotkeyAction = "start-hold" | "stop-hold" | "toggle" | "cancel";

export type HotkeyKeyEvent = {
  code: string;
  repeat?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
};

export type HotkeyTimers = {
  set: (fn: () => void, ms: number) => unknown;
  clear: (handle: unknown) => void;
  now: () => number;
};

/**
 * Pure hold/tap state machine. `onAction` performs the action; `keydown` and
 * `keyup` return `true` when the host should `preventDefault` the event.
 */
export class DictationHotkeyMachine {
  private pressedAt: number | null = null;
  private timer: unknown = null;
  private holding = false;
  private interrupted = false;
  private readonly hotkey: () => string;
  private readonly onAction: (action: HotkeyAction) => void;
  private readonly timers: HotkeyTimers;

  constructor(
    hotkey: () => string,
    onAction: (action: HotkeyAction) => void,
    timers: HotkeyTimers,
  ) {
    this.hotkey = hotkey;
    this.onAction = onAction;
    this.timers = timers;
  }

  keydown(event: HotkeyKeyEvent): boolean {
    if (event.code === this.hotkey()) {
      if (event.repeat) return this.pressedAt !== null;
      // AltGr on Windows arrives as ControlLeft + AltRight: a typing chord.
      if (event.ctrlKey || event.metaKey || event.shiftKey) return false;
      this.reset();
      this.pressedAt = this.timers.now();
      this.timer = this.timers.set(() => {
        this.timer = null;
        if (this.pressedAt === null || this.interrupted) return;
        this.holding = true;
        this.onAction("start-hold");
      }, HOLD_THRESHOLD_MS);
      return false;
    }
    if (this.pressedAt !== null) {
      // Another key while the dictation key is down: a typing chord.
      this.interrupted = true;
      this.clearTimer();
      if (this.holding) {
        this.holding = false;
        this.onAction("cancel");
      }
    }
    return false;
  }

  keyup(event: HotkeyKeyEvent): boolean {
    if (event.code !== this.hotkey() || this.pressedAt === null) return false;
    const heldFor = this.timers.now() - this.pressedAt;
    const wasHolding = this.holding;
    const interrupted = this.interrupted;
    this.reset();
    if (wasHolding) {
      this.onAction("stop-hold");
      return true;
    }
    if (!interrupted && heldFor < HOLD_THRESHOLD_MS) {
      this.onAction("toggle");
      return true;
    }
    return false;
  }

  /** Window lost focus mid-press: finish a hold, drop a pending tap. */
  blur(): void {
    const wasHolding = this.holding;
    this.reset();
    if (wasHolding) this.onAction("stop-hold");
  }

  private clearTimer() {
    if (this.timer !== null) this.timers.clear(this.timer);
    this.timer = null;
  }

  private reset() {
    this.clearTimer();
    this.pressedAt = null;
    this.holding = false;
    this.interrupted = false;
  }
}
