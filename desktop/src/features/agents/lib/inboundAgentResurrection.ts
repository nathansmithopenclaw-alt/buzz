import {
  CHANNEL_MESSAGE_EVENT_KINDS,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";
import type { ManagedAgent, RelayEvent } from "@/shared/api/types";

const INBOUND_KINDS = new Set<number>([
  ...CHANNEL_MESSAGE_EVENT_KINDS,
  KIND_HUDDLE_STARTED,
]);

export function eventMentionedPubkeys(event: RelayEvent): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const tag of event.tags) {
    if (tag[0] !== "p" || !tag[1]) continue;
    const pubkey = tag[1].toLowerCase();
    if (pubkey.length !== 64 || seen.has(pubkey)) continue;
    seen.add(pubkey);
    out.push(pubkey);
  }
  return out;
}

const liveListeners = new Set<(event: RelayEvent) => void>();

export function subscribeInboundLiveEvents(
  listener: (event: RelayEvent) => void,
): () => void {
  liveListeners.add(listener);
  return () => {
    liveListeners.delete(listener);
  };
}

export function notifyInboundLiveEvent(event: RelayEvent) {
  for (const listener of liveListeners) {
    listener(event);
  }
}

export function inboundResurrectionTargets(
  event: RelayEvent,
  agents: readonly Pick<ManagedAgent, "pubkey" | "backend" | "status">[],
): string[] {
  if (!INBOUND_KINDS.has(event.kind)) return [];
  const mentioned = new Set(eventMentionedPubkeys(event));
  if (mentioned.size === 0) return [];
  const author = event.pubkey.toLowerCase();
  return agents
    .filter(
      (agent) =>
        agent.backend.type === "local" &&
        agent.status !== "running" &&
        mentioned.has(agent.pubkey.toLowerCase()) &&
        agent.pubkey.toLowerCase() !== author,
    )
    .map((agent) => agent.pubkey);
}

const EXPECTED_WAKE_SKIPS = new Set([
  "SkipAlreadyRunning",
  "SkipInactive",
  "SkipNotLocal",
  "SkipSelfAuthored",
  "SkipAuthorGated",
  "unknown_agent",
]);

/** Keep policy/race skips quiet; surface every actionable spawn refusal. */
export function inboundWakeRefusalMessage(reason: string): string | null {
  const normalized = reason.trim();
  if (
    !normalized ||
    normalized === "spawned" ||
    EXPECTED_WAKE_SKIPS.has(normalized)
  ) {
    return null;
  }
  if (normalized === "SkipCrashBackoff") {
    return "This agent stopped waking automatically after repeated startup failures. Start it manually from Agents.";
  }
  return normalized;
}
