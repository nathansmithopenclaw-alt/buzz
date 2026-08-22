import type { RelayEvent } from "@/shared/api/types";
import { createTrailingDebounce } from "@/shared/lib/trailingDebounce";

type IncrementalBatch = {
  events: RelayEvent[];
  taskEvents: RelayEvent[];
  snapshotAt: number;
};

type DispatchLiveMergeDependencies<T> = {
  fold: (batch: IncrementalBatch) => Promise<T>;
  onFolded: (value: T) => void;
  onError: (error: unknown) => void;
  delayMs: number;
  now?: () => number;
  timerHost?: {
    setTimeout: (handler: () => void, milliseconds: number) => number;
    clearTimeout: (id: number) => void;
  };
};

export type DispatchLiveMerge = {
  push: (event: RelayEvent) => void;
  setColdLoadReady: (ready: boolean) => void;
  cancel: () => void;
};

/**
 * Batch live Dispatch events behind the initial history fold, then serialize
 * incremental Rust folds. The raw events remain the only renderer-owned state;
 * projection semantics stay inside the Tauri command.
 */
export function createDispatchLiveMerge<T>({
  fold,
  onFolded,
  onError,
  delayMs,
  now = () => Math.floor(Date.now() / 1_000),
  timerHost,
}: DispatchLiveMergeDependencies<T>): DispatchLiveMerge {
  const events = new Map<string, RelayEvent>();
  const taskEvents = new Map<string, RelayEvent>();
  let coldLoadReady = false;
  let cancelled = false;
  let queue = Promise.resolve();

  const flush = () => {
    if (cancelled || !coldLoadReady) return;
    if (events.size === 0 && taskEvents.size === 0) return;
    const batch: IncrementalBatch = {
      events: [...events.values()],
      taskEvents: [...taskEvents.values()],
      snapshotAt: now(),
    };
    events.clear();
    taskEvents.clear();

    const run = queue.then(() => fold(batch));
    queue = run.then(
      () => undefined,
      () => undefined,
    );
    void run.then(
      (value) => {
        if (!cancelled) onFolded(value);
      },
      (error: unknown) => {
        if (!cancelled) onError(error);
      },
    );
  };
  const debounce = createTrailingDebounce(flush, delayMs, timerHost);

  return {
    push: (event) => {
      (event.kind === 9 ? taskEvents : events).set(event.id, event);
      debounce.trigger();
    },
    setColdLoadReady: (ready) => {
      coldLoadReady = ready;
      if (ready && (events.size > 0 || taskEvents.size > 0)) debounce.trigger();
    },
    cancel: () => {
      cancelled = true;
      debounce.cancel();
      events.clear();
      taskEvents.clear();
    },
  };
}
