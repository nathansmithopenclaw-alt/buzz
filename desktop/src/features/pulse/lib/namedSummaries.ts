import { invokeTauri, signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import type { SummarySchedule } from "./summarySchedule";
import { configFromEvent } from "./summaryProtocol";
export type SummaryAudience =
  | { mode: "only_me" }
  | { mode: "community" }
  | { mode: "selected"; people: string[] };
export type NamedSummary = {
  v: 2;
  summary_id: string;
  name: string;
  reporter_pubkey: string;
  audience: SummaryAudience;
  read_delegation: boolean;
  channels: string[];
  dms: string[];
  exclude: string[];
  auto_include: boolean;
  schedule: Omit<SummarySchedule, "on"> | null;
  timezone: string;
  paused: boolean;
  archived: boolean;
};
export type SummaryDefinitionRecord = {
  event: RelayEvent;
  config: NamedSummary;
  legacy: boolean;
};
export type SummaryRunReceipt = {
  run_id: string;
  config_event_id: string;
  reporter_pubkey: string;
  audience: string[];
  started_at: string;
  expires_at: string;
  state: "queued" | "running" | "published" | "failed" | "needs_approval";
  report_event_id: string | null;
  error: string | null;
};
export type SummaryScope = { owner: string; relayUrl: string };
export function summaryOperation<T>(
  scope: SummaryScope,
  request: Record<string, unknown>,
): Promise<T> {
  return invokeTauri<T>("summary_operation", {
    request,
    expectedRelayUrl: scope.relayUrl,
    expectedSignerPubkey: scope.owner,
  });
}
export function definitionFromEvent(
  event: RelayEvent,
): SummaryDefinitionRecord | null {
  try {
    const value = JSON.parse(event.content);
    if (
      value.v === 2 &&
      typeof value.summary_id === "string" &&
      typeof value.name === "string" &&
      typeof value.reporter_pubkey === "string" &&
      Array.isArray(value.channels) &&
      Array.isArray(value.dms) &&
      ["only_me", "community", "selected"].includes(value.audience?.mode)
    )
      return { event, config: value, legacy: false };
    const old = configFromEvent(event);
    if (!old?.reporter_pubkey) return null;
    return {
      event,
      legacy: true,
      config: {
        v: 2,
        summary_id: "default",
        name: "Summary",
        reporter_pubkey: old.reporter_pubkey,
        audience: { mode: "only_me" },
        read_delegation: old.read_delegation === true,
        channels: old.channels,
        dms: old.dms,
        exclude: old.exclude,
        auto_include: old.auto_include,
        schedule:
          old.schedule === undefined
            ? { cron: `${old.minute} ${old.hour} * * *` }
            : old.schedule === null
              ? null
              : { cron: old.schedule.cron, interval: old.schedule.interval },
        timezone: old.timezone,
        paused: false,
        archived: false,
      },
    };
  } catch {
    return null;
  }
}
export async function fetchSummaryDefinitions(owner: string, relayUrl: string) {
  const events: RelayEvent[] = [];
  type Cursor = { before_time: number; before_id: string };
  let cursor: Cursor | null = null;
  do {
    const page: { events: RelayEvent[]; next: Cursor | null } =
      await summaryOperation(
        { owner, relayUrl },
        { summary_operation: "definitions", ...(cursor ?? {}) },
      );
    events.push(...page.events);
    if (cursor && page.next?.before_id === cursor.before_id)
      throw new Error(
        "Summary history did not advance. Refresh before continuing.",
      );
    cursor = page.next;
  } while (cursor);
  const unique = new Map<string, SummaryDefinitionRecord>();
  for (const event of events.sort(
    (a, b) => b.created_at - a.created_at || b.id.localeCompare(a.id),
  )) {
    const record = definitionFromEvent(event);
    if (record && !unique.has(record.config.summary_id))
      unique.set(record.config.summary_id, record);
  }
  return [...unique.values()];
}
export async function fetchSharedSummaryReports(
  owner: string,
  relayUrl: string,
) {
  const page = await summaryOperation<{ events: RelayEvent[] }>(
    { owner, relayUrl },
    {
      summary_operation: "reports_list",
      filter: { archived: false },
    },
  );
  return page.events;
}
export async function saveNamedSummary(
  scope: SummaryScope,
  config: NamedSummary,
  previous?: SummaryDefinitionRecord,
) {
  const support = await summaryOperation<{
    summary_protocol?: number;
    summary_hardening?: number;
  }>(scope, {
    summary_operation: "capabilities",
  });
  if (support?.summary_protocol !== 2 || support.summary_hardening !== 1)
    throw new Error(
      "Update this relay to save Summaries with reliable schedules and complete message coverage.",
    );
  const event = await signRelayEvent({
    kind: 30623,
    allowSelfTagging: true,
    createdAt: Math.max(
      Math.floor(Date.now() / 1000),
      (previous?.event.created_at ?? 0) + 1,
    ),
    content: JSON.stringify(config),
    tags: [
      [
        "d",
        config.summary_id === "default"
          ? "summary"
          : `summary:${config.summary_id}`,
      ],
      ["t", "buzz-brief"],
      ["summary", config.summary_id],
      [
        "previous",
        previous &&
        definitionFromEvent(previous.event)?.config.summary_id ===
          config.summary_id
          ? previous.event.id
          : "",
      ],
      ["p", scope.owner],
      ["p", config.reporter_pubkey],
    ],
  });
  if (event.pubkey !== scope.owner)
    throw new Error("Active identity changed. Summary was not saved.");
  await invokeTauri("publish_summary_definition", {
    event: JSON.stringify(event),
    expectedRelayUrl: scope.relayUrl,
    expectedSignerPubkey: scope.owner,
  });
}
