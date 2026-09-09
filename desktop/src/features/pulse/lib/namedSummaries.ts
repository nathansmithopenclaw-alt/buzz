import { dailyTimesToCrons } from "./summaryDailyTimes";
import { invokeTauri, signRelayEvent } from "@/shared/api/tauri";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import type { SummarySchedule } from "./summarySchedule";
import { configFromEvent } from "./summaryProtocol";
import {
  createWorkflow,
  updateWorkflow,
  deleteWorkflow,
  getChannelWorkflows,
} from "@/shared/api/tauriWorkflows";
import { openDm } from "@/shared/api/tauriChannels";

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
export async function fetchSummaryDefinitions(owner: string) {
  const events = await relayClient.fetchEvents({
    kinds: [30623],
    authors: [owner],
    "#p": [owner],
    "#t": ["buzz-brief"],
    limit: 200,
  });
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
  const support = await summaryOperation<{ summary_protocol?: number }>(scope, {
    summary_operation: "capabilities",
  });
  if (support?.summary_protocol !== 2)
    throw new Error(
      "This relay needs an update before it can save multiple or shared Summaries.",
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
  const previousIds =
    previous?.event.tags.filter((t) => t[0] === "workflow").map((t) => t[1]) ??
    [];
  if (config.archived || config.paused || !config.schedule) {
    for (const id of previousIds)
      await deleteWorkflow(id, {
        expectedRelayUrl: scope.relayUrl,
        expectedSignerPubkey: scope.owner,
      });
    return;
  }
  const dm = await openDm({
    pubkeys: [config.reporter_pubkey],
    expectedRelayUrl: scope.relayUrl,
    expectedSignerPubkey: scope.owner,
  });
  // Discover only workflows with this literal owner/summary binding. This also
  // recovers an accepted workflow whose config-link acknowledgement was lost.
  const candidates = (await getChannelWorkflows(dm.id)).filter(
    (w) =>
      w.ownerPubkey === scope.owner &&
      Array.isArray(w.definition.steps) &&
      w.definition.steps.some(
        (step: unknown) =>
          typeof step === "object" &&
          step !== null &&
          "action" in step &&
          step.action === "run_summary" &&
          "summary_id" in step &&
          step.summary_id === config.summary_id,
      ),
  );
  const expressions = config.schedule.dailyTimes
    ? dailyTimesToCrons(config.schedule.dailyTimes)
    : [config.schedule.cron ?? null];
  const ids: string[] = [];
  for (const [index, cron] of expressions.entries()) {
    const yaml = [
      `name: ${JSON.stringify(`Summary: ${config.name}`)}`,
      "trigger:",
      "  on: schedule",
      cron
        ? `  cron: ${JSON.stringify(cron)}`
        : `  interval: ${JSON.stringify(config.schedule.interval)}`,
      `  timezone: ${JSON.stringify(config.timezone)}`,
      "steps:",
      "  - id: run",
      "    action: run_summary",
      `    summary_id: ${JSON.stringify(config.summary_id)}`,
    ].join("\n");
    const existing = candidates[index];
    const saved = existing
      ? await updateWorkflow(existing.id, yaml, existing.revision, {
          expectedRelayUrl: scope.relayUrl,
          expectedSignerPubkey: scope.owner,
        })
      : await createWorkflow(dm.id, yaml, {
          expectedRelayUrl: scope.relayUrl,
          expectedSignerPubkey: scope.owner,
        });
    ids.push(saved.workflow.id);
  }
  for (const id of new Set([...previousIds, ...candidates.map((w) => w.id)]))
    if (!ids.includes(id))
      await deleteWorkflow(id, {
        expectedRelayUrl: scope.relayUrl,
        expectedSignerPubkey: scope.owner,
      });
  const linked = await signRelayEvent({
    kind: 30623,
    allowSelfTagging: true,
    createdAt: event.created_at + 1,
    content: event.content,
    tags: [...event.tags, ...ids.map((id) => ["workflow", id])],
  });
  if (linked.pubkey !== scope.owner)
    throw new Error("Active identity changed while saving the schedule.");
  await invokeTauri("publish_summary_definition", {
    event: JSON.stringify(linked),
    expectedRelayUrl: scope.relayUrl,
    expectedSignerPubkey: scope.owner,
  });
}
