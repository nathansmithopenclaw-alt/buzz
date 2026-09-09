import type { RelayEvent } from "@/shared/api/types";
import { KIND_BRIEF_CONFIG, KIND_BRIEF_REPORT } from "@/shared/constants/kinds";

const key = "buzz-e2e-summary-events";
type Filter = {
  kinds?: number[];
  authors?: string[];
  "#p"?: string[];
  "#d"?: string[];
  "#t"?: string[];
  limit?: number;
};
const isBrief = (kind: number) =>
  kind === KIND_BRIEF_CONFIG || kind === KIND_BRIEF_REPORT;

/** Minimal signed Summary persistence for browser acceptance; no agent execution is mocked as a report. */
export function handleMockSummaryFrame(
  type: string,
  rest: unknown[],
  owner: string,
  emit: (frame: unknown[]) => void,
): boolean {
  const events = (): RelayEvent[] =>
    JSON.parse(sessionStorage.getItem(key) ?? "[]");
  if (type === "EVENT") {
    const event = rest[0] as RelayEvent;
    if (!isBrief(event.kind)) return false;
    if (
      event.pubkey !== owner ||
      !event.tags.some((t) => t[0] === "p" && t[1] === owner)
    ) {
      emit(["OK", event.id, false, "Summary requires the owner recipient"]);
      return true;
    }
    sessionStorage.setItem(key, JSON.stringify([...events(), event]));
    emit(["OK", event.id, true, ""]);
    return true;
  }
  if (type !== "REQ") return false;
  const [subId, filter] = rest as [string, Filter];
  if (!filter.kinds?.length || !filter.kinds.every(isBrief)) return false;
  if (!filter["#p"]?.includes(owner)) {
    emit(["CLOSED", subId, "Summary requires a recipient filter"]);
    return true;
  }
  const matches = events().filter(
    (event) =>
      filter.kinds?.includes(event.kind) &&
      (!filter.authors || filter.authors.includes(event.pubkey)) &&
      (["p", "d", "t"] as const).every(
        (tag) =>
          !filter[`#${tag}`] ||
          event.tags.some(
            (t) => t[0] === tag && filter[`#${tag}`]?.includes(t[1]),
          ),
      ),
  );
  matches.sort(
    (a, b) => b.created_at - a.created_at || a.id.localeCompare(b.id),
  );
  for (const event of matches.slice(0, filter.limit ?? 50))
    emit(["EVENT", subId, event]);
  emit(["EOSE", subId]);
  return true;
}
