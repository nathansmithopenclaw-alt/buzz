import type { RelayEvent } from "@/shared/api/types";
import type { PendingEvent } from "@/shared/api/relayClientShared";
import {
  waitForRateLimit,
  activateRateLimitIfSignalled,
} from "@/shared/api/relayRateLimitGate";
import { PUBLISH_TIMEOUT_MS } from "@/shared/api/relayClientTimings";

type PublishSession = {
  generation: () => number;
  ownership: () => number;
  pendingEvents: Map<string, PendingEvent>;
  send: (payload: unknown[], generation: number) => Promise<void>;
  reconnect: () => Promise<number>;
  normalizeError: (error: unknown, fallback: string) => Error;
  recoverSocketFailure: (error: unknown, fallback: string) => Error;
};

/** Typed so no relay rejection text can be mistaken for a caller's cancel. */
export class PublishCanceledError extends Error {
  constructor() {
    super("Relay publish canceled by its caller.");
    this.name = "PublishCanceledError";
  }
}

/**
 * Publish once, with one reconnect retry, without crossing session ownership.
 * `isCurrent`, when given, is checked immediately before every socket send; a
 * false result rejects with `PublishCanceledError` and sends nothing.
 */
export async function publishSessionEvent(
  session: PublishSession,
  event: RelayEvent,
  timeoutMessage: string,
  sendErrorMessage: string,
  isCurrent?: () => boolean,
  onAcknowledged?: (message: string) => void,
): Promise<RelayEvent> {
  const publishOwnership = session.ownership();
  await waitForRateLimit();
  if (publishOwnership !== session.ownership()) {
    throw new Error("Relay disconnected for community switch.");
  }
  if (isCurrent?.() === false) throw new PublishCanceledError();
  const publishGeneration = session.generation();

  return new Promise<RelayEvent>((resolve, reject) => {
    const timeout = window.setTimeout(() => {
      session.pendingEvents.delete(event.id);
      reject(new Error(timeoutMessage));
    }, PUBLISH_TIMEOUT_MS);
    const pendingEvent = { event, resolve, reject, timeout, onAcknowledged };
    session.pendingEvents.set(event.id, pendingEvent);

    void session
      .send(["EVENT", event], publishGeneration)
      .catch(async (error) => {
        // A disconnect may already have rejected this operation while the send
        // was in flight. Its late failure must not reset the replacement session.
        if (
          publishOwnership !== session.ownership() ||
          publishGeneration !== session.generation() ||
          session.pendingEvents.get(event.id) !== pendingEvent
        ) {
          return;
        }

        // Expected socket recovery must not reject the operation being retried.
        session.pendingEvents.delete(event.id);
        const sendError = session.recoverSocketFailure(error, sendErrorMessage);
        session.pendingEvents.set(event.id, pendingEvent);
        let retryGeneration: number | null = null;

        try {
          retryGeneration = await session.reconnect();
          if (
            publishOwnership !== session.ownership() ||
            session.generation() !== retryGeneration ||
            session.pendingEvents.get(event.id) !== pendingEvent
          ) {
            throw new Error(
              "Relay publish was superseded by a session change.",
            );
          }
          if (isCurrent?.() === false) {
            window.clearTimeout(timeout);
            session.pendingEvents.delete(event.id);
            reject(new PublishCanceledError());
            return;
          }
          await session.send(["EVENT", event], retryGeneration);
        } catch (retryError) {
          if (session.pendingEvents.get(event.id) !== pendingEvent) return;

          window.clearTimeout(timeout);
          session.pendingEvents.delete(event.id);
          reject(
            publishOwnership === session.ownership() &&
              retryGeneration !== null &&
              session.generation() === retryGeneration
              ? session.recoverSocketFailure(retryError, sendError.message)
              : session.normalizeError(retryError, sendError.message),
          );
        }
      });
  });
}

/** Return the relay ACK payload for a read-only Dispatch command. */
export async function readDispatchResponse(
  event: RelayEvent,
  publish: (onAcknowledged: (message: string) => void) => Promise<RelayEvent>,
): Promise<string> {
  if (event.kind !== 40028)
    throw new Error("Expected a read-only Dispatch command.");
  let response: string | undefined;
  await publish((message) => {
    response = message;
  });
  if (response === undefined)
    throw new Error("The relay did not return a command response.");
  return response;
}

/** Settle one pending publish from the authoritative relay ACK. */
export function settlePublishedEvent(
  pendingEvents: Map<string, PendingEvent>,
  eventId: string,
  success: boolean,
  message: string,
) {
  const pendingEvent = pendingEvents.get(eventId);
  if (!pendingEvent) {
    return;
  }

  window.clearTimeout(pendingEvent.timeout);
  pendingEvents.delete(eventId);

  if (success) {
    pendingEvent.onAcknowledged?.(message);
    pendingEvent.resolve(pendingEvent.event);
  } else {
    // Back-pressure now arrives here rather than as a NOTICE: the relay
    // rejects an over-quota EVENT on the OK channel so this pending publish
    // can be settled at all. Unarmed, the send retries into the same quota.
    activateRateLimitIfSignalled(message);
    pendingEvent.reject(new Error(message || "Relay rejected the event."));
  }
}
