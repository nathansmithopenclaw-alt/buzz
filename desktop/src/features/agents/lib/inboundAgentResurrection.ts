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

const OBSERVER_REFUSAL =
  "Observer device cannot control agents; switch this device to Host in Settings";
const ROLE_CHOICE_REFUSAL =
  "Choose whether this device is a Host or Observer before controlling agents";
const WAKE_REFUSAL_COOLDOWN_MS = 60 * 60 * 1_000;
const WAKE_REFUSAL_LIMIT = 2_000;
const wakeRefusalShownAt = new Map<string, number>();

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
  if (normalized === OBSERVER_REFUSAL) {
    return "This device is an Observer, so it cannot wake agents. Switch to Host in Settings to run agents here.";
  }
  if (normalized === ROLE_CHOICE_REFUSAL) {
    return "Choose whether this device is a Host or Observer before waking agents.";
  }
  return "This agent did not wake automatically. Start it manually from Agents.";
}

/** Limit repeated refusals from a busy channel to once per agent and reason. */
export function shouldShowInboundWakeRefusal(
  pubkey: string,
  reason: string,
  now = Date.now(),
): boolean {
  const key = `${pubkey.toLowerCase()}:${reason.trim()}`;
  const lastShown = wakeRefusalShownAt.get(key);
  if (lastShown !== undefined && now - lastShown < WAKE_REFUSAL_COOLDOWN_MS) {
    return false;
  }
  wakeRefusalShownAt.delete(key);
  wakeRefusalShownAt.set(key, now);
  if (wakeRefusalShownAt.size > WAKE_REFUSAL_LIMIT) {
    const oldest = wakeRefusalShownAt.keys().next().value;
    if (oldest) wakeRefusalShownAt.delete(oldest);
  }
  return true;
}
