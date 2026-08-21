import type { ObserverEvent } from "./ui/agentSessionTypes";

export function compareObserverEvents(
  left: ObserverEvent,
  right: ObserverEvent,
) {
  const leftTime = Date.parse(left.timestamp);
  const rightTime = Date.parse(right.timestamp);
  if (Number.isFinite(leftTime) && Number.isFinite(rightTime)) {
    const timeDiff = leftTime - rightTime;
    if (timeDiff !== 0) return timeDiff;
  }
  return left.seq - right.seq;
}

/** Use the same timestamp-then-sequence ordering as transcript sorting. */
export function isObserverEventAfter(
  candidate: { timestamp: string; seq: number },
  stored: { timestamp: string; seq: number },
): boolean {
  const candidateTime = Date.parse(candidate.timestamp);
  const storedTime = Date.parse(stored.timestamp);
  if (
    Number.isFinite(candidateTime) &&
    Number.isFinite(storedTime) &&
    candidateTime !== storedTime
  ) {
    return candidateTime > storedTime;
  }
  return candidate.seq > stored.seq;
}
