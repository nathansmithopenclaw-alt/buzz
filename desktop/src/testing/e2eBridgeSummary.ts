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

/** Browser-only control receipts; these never fabricate a published report. */
export function handleMockSummaryCommand(
  command: string,
  payload: Record<string, unknown>,
  owner: string,
): unknown {
  if (command === "publish_summary_definition") {
    const event = JSON.parse(String(payload.event)) as RelayEvent;
    if (event.pubkey !== owner) throw new Error("Summary owner mismatch");
    const events: RelayEvent[] = JSON.parse(
      sessionStorage.getItem(key) ?? "[]",
    );
    sessionStorage.setItem(key, JSON.stringify([...events, event]));
    return { accepted: true, event_id: event.id, message: "" };
  }
  const request = payload.request as {
    summary_operation: string;
    summary_id?: string;
    run_id?: string;
  };
  if (request.summary_operation === "capabilities")
    return { summary_protocol: 2 };
  if (request.summary_operation === "people")
    return {
      people: [owner, "33".repeat(32), "44".repeat(32)],
      complete: true,
    };
  const runs = JSON.parse(
    sessionStorage.getItem("buzz-e2e-summary-runs") ?? "{}",
  );
  if (request.summary_operation === "history")
    return {
      runs: runs[request.summary_id ?? ""]
        ? [runs[request.summary_id ?? ""]]
        : [],
    };
  if (request.summary_operation === "start") {
    const id = request.summary_id ?? "";
    if (runs[id]) return { run: runs[id], created: false };
    const events: RelayEvent[] = JSON.parse(
      sessionStorage.getItem(key) ?? "[]",
    );
    const event = events
      .filter((e) => JSON.parse(e.content).summary_id === id)
      .at(-1);
    if (!event) throw new Error("Missing Summary");
    const config = JSON.parse(event.content);
    const run = {
      run_id: crypto.randomUUID(),
      config_event_id: event.id,
      reporter_pubkey: config.reporter_pubkey,
      audience: [owner],
      started_at: new Date().toISOString(),
      expires_at: new Date(Date.now() + 1800000).toISOString(),
      state: "running",
      report_event_id: null,
      error: null,
    };
    runs[id] = run;
    sessionStorage.setItem("buzz-e2e-summary-runs", JSON.stringify(runs));
    return { run, created: true };
  }
  throw new Error("Unsupported mock Summary operation");
}
