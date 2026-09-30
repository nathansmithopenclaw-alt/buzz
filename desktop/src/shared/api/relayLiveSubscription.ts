import type { RelayEvent } from "./types";
import type {
  LiveSubscriptionReadiness,
  RelaySubscription,
  RelaySubscriptionFilter,
} from "./relayClientShared";
import { clearClosedRetry } from "./relayClosedRecovery";

type LiveSession = {
  epoch: () => number;
  signal: AbortSignal;
  subscriptions: Map<string, RelaySubscription>;
  cancelQueued: (id: string) => void;
  close: (id: string) => Promise<void>;
  ensureConnected: () => Promise<number>;
  send: (
    payload: unknown[],
    fallback: string,
    onDispatch: () => void,
  ) => Promise<void>;
};

/** Register a cancellable live subscription without crossing workspace ownership. */
export async function subscribeLiveSession(
  session: LiveSession,
  filter: RelaySubscriptionFilter,
  onEvent: (event: RelayEvent) => void,
  onReady?: (readiness: LiveSubscriptionReadiness) => void,
  readinessTimeoutMs = 250,
  signal?: AbortSignal,
  priority?: "interactive",
  onTerminalClosed?: (message: string) => void,
) {
  const epoch = session.epoch();
  const sessionSignal = session.signal;
  signal?.throwIfAborted();
  const subId = `live-${crypto.randomUUID()}`;
  let fallbackTimeout: number | undefined;
  let settleReady = () => {};
  const onRemoved = () => {
    signal?.removeEventListener("abort", abort);
    sessionSignal.removeEventListener("abort", abort);
    window.clearTimeout(fallbackTimeout);
    settleReady();
  };
  const subscription: Extract<RelaySubscription, { mode: "live" }> = {
    mode: "live",
    filter,
    priority,
    onEvent,
    onRemoved,
    onTerminalClosed,
  };
  const dispose = async () => {
    if (session.subscriptions.get(subId) !== subscription) return;
    session.subscriptions.delete(subId);
    session.cancelQueued(subId);
    clearClosedRetry(subscription);
    onRemoved();
    // Workspace teardown closes its socket; never send on its replacement.
    if (epoch === session.epoch()) await session.close(subId);
  };
  let rejectCancelled = (_error: Error) => {};
  const cancelled = new Promise<never>((_resolve, reject) => {
    rejectCancelled = reject;
  });
  const abort = () => {
    rejectCancelled(
      new DOMException("Live subscription cancelled.", "AbortError"),
    );
    void dispose().catch(() => {});
    onRemoved();
  };
  signal?.addEventListener("abort", abort, { once: true });
  sessionSignal.addEventListener("abort", abort, { once: true });
  try {
    await Promise.race([session.ensureConnected(), cancelled]);
    signal?.throwIfAborted();
    sessionSignal.throwIfAborted();
    const ready = new Promise<void>((resolve) => {
      settleReady = resolve;
    });
    let readySettled = false;
    subscription.resolveReady = (readiness) => {
      if (readySettled) return;
      readySettled = true;
      window.clearTimeout(fallbackTimeout);
      onReady?.(readiness);
      settleReady();
    };
    session.subscriptions.set(subId, subscription);
    await Promise.race([
      session.send(
        ["REQ", subId, filter],
        "Failed to restore relay subscription.",
        () => {
          if (readySettled) return;
          window.clearTimeout(fallbackTimeout);
          fallbackTimeout = window.setTimeout(
            () => subscription.resolveReady?.("timeout"),
            readinessTimeoutMs,
          );
        },
      ),
      cancelled,
    ]);
    await Promise.race([ready, cancelled]);
    return dispose;
  } catch (error) {
    await dispose().catch(() => {});
    onRemoved();
    throw error;
  }
}
