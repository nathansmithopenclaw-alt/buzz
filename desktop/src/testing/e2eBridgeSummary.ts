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
    metadata_only?: boolean;
    source_report?: string;
    state?: string;
    message?: string;
    event_ids?: string[];
    filter?: {
      archived: boolean;
      query?: string;
      owner?: string;
      summary_id?: string;
      reporter?: string;
      since?: number;
      until?: number;
      before_time?: number;
      before_id?: string;
    };
  };
  if (request.summary_operation === "definitions") {
    const events: RelayEvent[] = JSON.parse(
      sessionStorage.getItem(key) ?? "[]",
    );
    const current = new Map<string, RelayEvent>();
    for (const event of events
      .filter((e) => e.kind === 30623 && e.pubkey === owner)
      .sort(
        (a, b) => b.created_at - a.created_at || b.id.localeCompare(a.id),
      )) {
      const id = JSON.parse(event.content).summary_id ?? "default";
      if (!current.has(id)) current.set(id, event);
    }
    return { events: [...current.values()], next: null };
  }
  if (request.summary_operation === "overview") {
    const events: RelayEvent[] = JSON.parse(
      sessionStorage.getItem(key) ?? "[]",
    );
    const archived: string[] = JSON.parse(
      sessionStorage.getItem(`buzz-e2e-summary-archive:${owner}`) ?? "[]",
    );
    const runs = JSON.parse(
      sessionStorage.getItem("buzz-e2e-summary-runs") ?? "{}",
    );
    const ids = new Set(
      events
        .filter((e) => e.kind === KIND_BRIEF_CONFIG && e.pubkey === owner)
        .map((e) => JSON.parse(e.content).summary_id ?? "default"),
    );
    return {
      summaries: [...ids].map((id) => {
        const reports = events.filter(
          (e) =>
            e.kind === KIND_BRIEF_REPORT &&
            !archived.includes(e.id) &&
            e.tags.some((t) => t[0] === "summary" && t[1] === id) &&
            e.tags.some((t) => t[0] === "owner" && t[1] === owner),
        );
        const run = runs[String(id)];
        return {
          summary_id: id,
          report_count: reports.length,
          latest_at: reports.length
            ? Math.max(...reports.map((e) => e.created_at))
            : null,
          state: run?.state ?? null,
          error: run?.error ?? null,
          last_run_at: run?.started_at ?? null,
        };
      }),
    };
  }
  if (request.summary_operation.startsWith("reports_")) {
    const archivedKey = `buzz-e2e-summary-archive:${owner}`;
    const archived: string[] = JSON.parse(
      sessionStorage.getItem(archivedKey) ?? "[]",
    );
    const events: RelayEvent[] = JSON.parse(
      sessionStorage.getItem(key) ?? "[]",
    );
    if (request.summary_operation === "reports_get") {
      const event = events.find(
        (e) =>
          e.id === request.event_ids?.[0] &&
          e.kind === KIND_BRIEF_REPORT &&
          e.tags.some((t) => t[0] === "p" && t[1] === owner),
      );
      if (!event) throw new Error("Report is unavailable");
      return { event, report: mockReportMetadata(event, events, owner) };
    }
    if (request.summary_operation === "reports_list") {
      const f = request.filter ?? { archived: false };
      const rows = events
        .filter(
          (e) =>
            e.kind === 40030 &&
            e.tags.some((t) => t[0] === "p" && t[1] === owner),
        )
        .filter(
          (e) =>
            archived.includes(e.id) === f.archived &&
            (!f.query ||
              [
                e.content,
                ...(["summary_name", "owner_name", "agent_name"] as const).map(
                  (key) => String(mockReportMetadata(e, events, owner)[key]),
                ),
              ]
                .join(" ")
                .toLowerCase()
                .includes(f.query.toLowerCase())) &&
            (!f.summary_id ||
              e.tags.some(
                (t) => t[0] === "summary" && t[1] === f.summary_id,
              )) &&
            (!f.owner ||
              e.tags.some((t) => t[0] === "owner" && t[1] === f.owner)) &&
            (!f.reporter || e.pubkey === f.reporter) &&
            (!f.since || e.created_at >= f.since) &&
            (!f.until || e.created_at <= f.until) &&
            (!f.before_time ||
              e.created_at < f.before_time ||
              (e.created_at === f.before_time && e.id < (f.before_id ?? ""))),
        )
        .sort(
          (a, b) => b.created_at - a.created_at || b.id.localeCompare(a.id),
        );
      const page = rows.slice(0, 50);
      const last = page.at(-1);
      return {
        ...(request.metadata_only
          ? {
              reports: page.map((e) => {
                return mockReportMetadata(e, events, owner);
              }),
            }
          : { events: page }),
        next:
          rows.length > 50 && last
            ? { before_time: last.created_at, before_id: last.id }
            : null,
      };
    }
    const ids = request.event_ids ?? [];
    if (request.summary_operation === "reports_delete") {
      sessionStorage.setItem(
        key,
        JSON.stringify(events.filter((e) => !ids.includes(e.id))),
      );
    } else {
      sessionStorage.setItem(
        archivedKey,
        JSON.stringify(
          request.summary_operation === "reports_archive"
            ? [...new Set([...archived, ...ids])]
            : archived.filter((id) => !ids.includes(id)),
        ),
      );
    }
    return { updated: ids.length };
  }
  if (request.summary_operation === "capabilities")
    return {
      summary_protocol: 2,
      summary_report_archive: 1,
      summary_hardening: 1,
    };
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
  if (request.summary_operation === "progress") {
    const run = runs[request.summary_id ?? ""];
    if (!run || run.run_id !== request.run_id)
      throw new Error("Run is unavailable");
    run.state = request.state;
    run.error = request.message;
    sessionStorage.setItem("buzz-e2e-summary-runs", JSON.stringify(runs));
    return { run };
  }
  if (["start", "dispatch"].includes(request.summary_operation)) {
    const id = request.summary_id ?? "";
    if (
      runs[id] &&
      ["running", "queued", "needs_approval"].includes(runs[id].state)
    )
      return { run: runs[id], created: false };
    const events: RelayEvent[] = JSON.parse(
      sessionStorage.getItem(key) ?? "[]",
    );
    const event = events
      .filter((e) => JSON.parse(e.content).summary_id === id)
      .at(-1);
    if (!event) throw new Error("Missing Summary");
    const config = JSON.parse(event.content);
    const original = events.find((e) => e.id === request.source_report);
    const originalContent = original ? JSON.parse(original.content) : null;
    const run = {
      replay_period: originalContent
        ? {
            source_report: original?.id,
            since: originalContent.since,
            until: originalContent.until,
          }
        : null,
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
    return { run, created: true, event_id: "mock-summary-request" };
  }
  throw new Error("Unsupported mock Summary operation");
}

function mockReportMetadata(
  e: RelayEvent,
  events: RelayEvent[],
  owner: string,
) {
  const content = JSON.parse(e.content);
  const reportOwner = e.tags.find((t) => t[0] === "owner")?.[1] ?? owner;
  const id = e.tags.find((t) => t[0] === "summary")?.[1] ?? "default";
  const definition = events
    .filter(
      (v) =>
        v.kind === KIND_BRIEF_CONFIG &&
        v.pubkey === reportOwner &&
        (JSON.parse(v.content).summary_id ?? "default") === id,
    )
    .at(-1);
  return {
    id: e.id,
    pubkey: e.pubkey,
    created_at: e.created_at,
    title: content.title ?? "Summary report",
    owner: reportOwner,
    summary_id: id,
    summary_name: definition
      ? JSON.parse(definition.content).name
      : "Shared Summary",
    agent_name: "Summary agent",
    owner_name: reportOwner === owner ? "You" : "Report owner",
    can_delete: reportOwner === owner,
    since: content.since,
    until: content.until,
  };
}
