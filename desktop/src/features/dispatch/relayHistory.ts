import { relayClient } from "@/shared/api/relayClient";
import {
  activateRateLimit,
  isRateLimited,
  parseRateLimitHint,
  waitForRateLimit,
} from "@/shared/api/relayRateLimitGate";
import { createDispatchHistoryClient } from "./history";

export function dispatchHistoryClient(
  onRetryingChange?: (value: boolean) => void,
  signal?: AbortSignal,
) {
  return createDispatchHistoryClient({
    signal,
    fetchEvents: (filter) => relayClient.fetchEvents(filter),
    fetchEventsMulti: (filters) => relayClient.fetchEventsMulti(filters),
    waitForRateLimit,
    isRateLimited,
    activateRateLimit: (message) =>
      activateRateLimit(parseRateLimitHint(message)),
    onRetryingChange,
  });
}
