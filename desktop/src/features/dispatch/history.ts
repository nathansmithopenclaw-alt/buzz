import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";

const PAGE_SIZE = 500;
const DEFAULT_MAX_RATE_LIMIT_RETRIES = 3;
const DEFAULT_REQUEST_INTERVAL_MS = 150;

type ExhaustiveFilter = Omit<RelaySubscriptionFilter, "limit" | "until">;

type DispatchHistoryDependencies = {
  fetchEvents: (filter: RelaySubscriptionFilter) => Promise<RelayEvent[]>;
  waitForRateLimit: () => Promise<void>;
  isRateLimited: () => boolean;
  activateRateLimit: (message: string) => void;
  onRetryingChange?: (retrying: boolean) => void;
  maxRateLimitRetries?: number;
  requestIntervalMs?: number;
  now?: () => number;
  sleep?: (milliseconds: number) => Promise<void>;
};

export type DispatchHistoryClient = {
  fetch: (filter: RelaySubscriptionFilter) => Promise<RelayEvent[]>;
  fetchExhaustively: (filter: ExhaustiveFilter) => Promise<RelayEvent[]>;
};

export class DispatchHistoryGapError extends Error {
  constructor() {
    super("Tracker history cannot be exhausted at the timestamp boundary.");
    this.name = "DispatchHistoryGapError";
  }
}

export function isDispatchHistoryGap(error: unknown): boolean {
  return error instanceof DispatchHistoryGapError;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function isRateLimitRejection(error: unknown): boolean {
  return /^(?:relay )?rate-limited:/i.test(errorMessage(error).trim());
}

/**
 * Build one cold-load client whose requests share a serial queue and pacing
 * clock. A rate-limited page keeps its queue slot while it waits and retries,
 * preventing sibling Dispatch streams from stampeding the same relay window.
 */
export function createDispatchHistoryClient({
  fetchEvents,
  waitForRateLimit,
  isRateLimited,
  activateRateLimit,
  onRetryingChange,
  maxRateLimitRetries = DEFAULT_MAX_RATE_LIMIT_RETRIES,
  requestIntervalMs = DEFAULT_REQUEST_INTERVAL_MS,
  now = Date.now,
  sleep = (milliseconds) =>
    new Promise((resolve) => globalThis.setTimeout(resolve, milliseconds)),
}: DispatchHistoryDependencies): DispatchHistoryClient {
  let queue = Promise.resolve();
  let lastRequestStartedAt: number | null = null;
  let retrying = false;

  const setRetrying = (next: boolean) => {
    if (next === retrying) return;
    retrying = next;
    onRetryingChange?.(next);
  };

  const waitForRequestSlot = async () => {
    if (lastRequestStartedAt !== null) {
      const remaining = requestIntervalMs - (now() - lastRequestStartedAt);
      if (remaining > 0) await sleep(remaining);
    }
    lastRequestStartedAt = now();
  };

  const requestWithRetry = async (filter: RelaySubscriptionFilter) => {
    let rateLimitRetries = 0;
    for (;;) {
      if (isRateLimited()) setRetrying(true);
      await waitForRateLimit();
      await waitForRequestSlot();
      try {
        const events = await fetchEvents(filter);
        setRetrying(false);
        return events;
      } catch (error) {
        if (
          !isRateLimitRejection(error) ||
          rateLimitRetries >= maxRateLimitRetries
        ) {
          setRetrying(false);
          throw error;
        }
        rateLimitRetries += 1;
        setRetrying(true);
        activateRateLimit(errorMessage(error));
      }
    }
  };

  const fetch = (filter: RelaySubscriptionFilter) => {
    const request = queue.then(
      () => requestWithRetry(filter),
      () => requestWithRetry(filter),
    );
    queue = request.then(
      () => undefined,
      () => undefined,
    );
    return request;
  };

  const fetchExhaustively = async (
    filter: ExhaustiveFilter,
  ): Promise<RelayEvent[]> => {
    const events = new Map<string, RelayEvent>();
    let until: number | undefined;
    for (;;) {
      const page = await fetch({
        ...filter,
        limit: PAGE_SIZE,
        ...(until === undefined ? {} : { until }),
      });
      for (const event of page) events.set(event.id, event);
      if (page.length < PAGE_SIZE) return [...events.values()];
      const oldest = Math.min(...page.map((event) => event.created_at));
      const boundary = await fetch({
        ...filter,
        limit: PAGE_SIZE,
        since: oldest,
        until: oldest,
      });
      for (const event of boundary) events.set(event.id, event);
      if (boundary.length >= PAGE_SIZE) {
        throw new DispatchHistoryGapError();
      }
      if (filter.since !== undefined && oldest <= filter.since) {
        return [...events.values()];
      }
      if (oldest <= 0) return [...events.values()];
      until = oldest - 1;
    }
  };

  return { fetch, fetchExhaustively };
}
