import type { RelayEvent } from "@/shared/api/types";

type Dispose = () => Promise<void>;
type TimerHost = Pick<Window, "setTimeout" | "clearTimeout">;

/** Keep the owner observer listener alive without requiring a WebView reload. */
export function startObserverSubscription({
  subscribe,
  onReconnect,
  onEvent,
  onState,
  timerHost = {
    setTimeout: (handler, ms) => window.setTimeout(handler, ms),
    clearTimeout: (id) => window.clearTimeout(id),
  },
}: {
  subscribe: (
    onEvent: (event: RelayEvent) => void,
    onTerminalClosed: (message: string) => void,
  ) => Promise<Dispose>;
  onReconnect: (listener: () => void) => () => void;
  onEvent: (event: RelayEvent) => void;
  onState: (
    state: "connecting" | "open" | "error",
    error: string | null,
  ) => void;
  timerHost?: TimerHost;
}) {
  let stopped = false;
  let running = false;
  let rerun = false;
  let active: Dispose | undefined;
  let retryTimer: number | undefined;
  let retryAttempt = 0;
  let attempt = 0;

  const clearRetry = () => {
    if (retryTimer !== undefined) timerHost.clearTimeout(retryTimer);
    retryTimer = undefined;
  };

  const dispose = (cleanup: Dispose | undefined) => {
    void cleanup?.().catch((error) => {
      console.error("Observer subscription cleanup failed:", error);
    });
  };

  const scheduleRetry = () => {
    if (stopped || retryTimer !== undefined) return;
    const delay = Math.min(1_000 * 2 ** retryAttempt, 30_000);
    retryAttempt = Math.min(retryAttempt + 1, 5);
    retryTimer = timerHost.setTimeout(() => {
      retryTimer = undefined;
      void run();
    }, delay);
  };

  async function run() {
    if (stopped || active) return;
    if (running) {
      rerun = true;
      return;
    }
    running = true;
    const currentAttempt = ++attempt;
    let closed = false;
    onState("connecting", null);
    try {
      const cleanup = await subscribe(
        (event) => {
          if (!stopped && !closed && currentAttempt === attempt) onEvent(event);
        },
        (message) => {
          if (stopped || closed || currentAttempt !== attempt) return;
          closed = true;
          active = undefined; // RelayClient has removed this subscription.
          onState("error", message || "Observer relay subscription closed.");
          scheduleRetry();
        },
      );
      // CLOSED can arrive before subscribeLive resolves its readiness promise.
      // Never cache that dead handle or report it as open.
      if (stopped || closed) {
        dispose(cleanup);
      } else {
        active = cleanup;
        retryAttempt = 0;
        clearRetry();
        onState("open", null);
      }
    } catch (error) {
      if (!stopped) {
        onState(
          "error",
          error instanceof Error
            ? error.message
            : "Observer subscription failed.",
        );
        scheduleRetry();
      }
    } finally {
      running = false;
      if (rerun && !stopped) {
        rerun = false;
        void run();
      }
    }
  }

  const unsubscribeReconnect = onReconnect(() => {
    if (stopped || active) return; // Healthy live subscriptions replay themselves.
    clearRetry();
    void run();
  });

  return {
    ready: run(),
    stop: () => {
      stopped = true;
      clearRetry();
      unsubscribeReconnect();
      dispose(active);
      active = undefined;
    },
  };
}
