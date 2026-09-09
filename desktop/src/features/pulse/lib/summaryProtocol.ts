import { openDm } from "@/shared/api/tauriChannels";
import { buildDirectMessageIntro } from "@/features/channels/lib/dmParticipantDisplay";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { relayClient } from "@/shared/api/relayClient";
import {
  addChannelMembers,
  removeChannelMember,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { KIND_BRIEF_CONFIG, KIND_BRIEF_REPORT } from "@/shared/constants/kinds";

import {
  syncSummarySchedule,
  type SummarySchedule,
  summaryScheduleError,
} from "./summarySchedule";
import type {
  SummaryConversation,
  SummarySourceConfig,
} from "./summarySources";
import { defaultSourceConfig, listsFromChecked } from "./summarySources";

export const BRIEF_DISCOVERY_TAG = "buzz-brief";
export const BRIEF_CONFIG_D_TAG = "summary";

export type BriefConfigPayload = {
  v: 1;
  read_delegation?: boolean;
  /** Absent on legacy daily setups; null means manual only. */
  schedule?: SummarySchedule | null;
  reporter_pubkey: string | null;
  hour: number;
  minute: number;
  timezone: string;
  auto_include: boolean;
  channels: string[];
  dms: string[];
  exclude: string[];
};

export type BriefReportPayload = {
  v: 1;
  run_id: string;
  title: string;
  markdown: string;
  status: unknown;
  period: { since: number; until: number };
  sources: { channels: string[]; dms: string[] };
};

export type SummaryReport = {
  id: string;
  createdAt: number;
  pubkey: string;
  report: BriefReportPayload;
};

function tagValue(event: RelayEvent, name: string): string | undefined {
  return event.tags.find((tag) => tag[0] === name)?.[1];
}

export function conversationsFromChannels(
  channels: Channel[],
  currentPubkey?: string,
  profiles?: UserProfileLookup,
): SummaryConversation[] {
  return channels
    .filter((channel) => channel.isMember)
    .map((channel) => ({
      id: channel.id,
      name:
        channel.channelType === "dm"
          ? buildDirectMessageIntro({ channel, currentPubkey, profiles })
              ?.displayName ||
            channel.participants.join(", ") ||
            "Direct message"
          : channel.name,
      kind: channel.channelType === "dm" ? "dm" : "channel",
      archived: Boolean(channel.archivedAt),
    }));
}

export function configFromEvent(event: RelayEvent): BriefConfigPayload | null {
  try {
    const parsed = JSON.parse(event.content) as BriefConfigPayload;
    if (parsed.v !== 1) return null;
    return parsed;
  } catch {
    return null;
  }
}

export function reportFromEvent(
  event: RelayEvent,
  expectedReporterPubkey?: string | null,
): SummaryReport | null {
  try {
    const parsed = JSON.parse(event.content) as BriefReportPayload;
    if (parsed.v !== 1 || tagValue(event, "t") !== BRIEF_DISCOVERY_TAG) {
      return null;
    }
    if (
      expectedReporterPubkey &&
      event.pubkey.toLowerCase() !== expectedReporterPubkey.toLowerCase()
    ) {
      return null;
    }
    return {
      id: event.id,
      createdAt: event.created_at,
      pubkey: event.pubkey,
      report: parsed,
    };
  } catch {
    return null;
  }
}

export function workflowIdFromEvent(event: RelayEvent): string | null {
  return tagValue(event, "workflow") ?? null;
}

export function membershipDiff(
  previous: readonly string[],
  next: readonly string[],
): { added: string[]; removed: string[] } {
  const nextSet = new Set(next.map((id) => id.toLowerCase()));
  const previousSet = new Set(previous.map((id) => id.toLowerCase()));
  return {
    added: next.filter((id) => !previousSet.has(id.toLowerCase())),
    removed: previous.filter((id) => !nextSet.has(id.toLowerCase())),
  };
}

export function sourceConfigFromPayload(
  payload: BriefConfigPayload | null,
  conversations: SummaryConversation[],
): SummarySourceConfig {
  if (!payload) return defaultSourceConfig(conversations);
  return {
    autoInclude: payload.auto_include,
    include: [...payload.channels, ...payload.dms],
    exclude: [...payload.exclude],
  };
}

export function briefConfigFilter(pubkey: string) {
  return {
    kinds: [KIND_BRIEF_CONFIG],
    authors: [pubkey],
    "#d": [BRIEF_CONFIG_D_TAG],
    "#p": [pubkey],
    limit: 1,
  };
}

export function briefReportsFilter(
  ownerPubkey: string,
  reporterPubkey: string,
) {
  return {
    kinds: [KIND_BRIEF_REPORT],
    authors: [reporterPubkey],
    "#p": [ownerPubkey],
    "#t": [BRIEF_DISCOVERY_TAG],
    limit: 50,
  };
}

export async function fetchBriefConfig(
  pubkey: string,
): Promise<RelayEvent | null> {
  const events = await relayClient.fetchEvents(briefConfigFilter(pubkey));
  return events[0] ?? null;
}

export async function fetchBriefReports(
  pubkey: string,
  reporterPubkey?: string | null,
): Promise<SummaryReport[]> {
  if (!reporterPubkey) return [];
  const events = await relayClient.fetchEvents(
    briefReportsFilter(pubkey, reporterPubkey),
  );
  return events
    .map((event) => reportFromEvent(event, reporterPubkey))
    .filter((report): report is SummaryReport => report !== null)
    .sort((a, b) => b.createdAt - a.createdAt || b.id.localeCompare(a.id));
}

export async function publishBriefConfig(input: {
  ownerPubkey: string;
  hour: number;
  minute: number;
  timezone: string;
  autoInclude: boolean;
  reporterPubkey: string | null;
  reporterName: string;
  conversations: SummaryConversation[];
  sourceConfig: SummarySourceConfig;
  schedule?: SummarySchedule | null;
  previousConfigCreatedAt?: number;
  previousChannelIds?: string[];
  previousWorkflowId?: string | null;
}): Promise<void> {
  const scheduleError = summaryScheduleError(input.schedule ?? null);
  if (scheduleError) throw new Error(scheduleError);
  const lists = listsFromChecked(input.conversations, input.sourceConfig);
  if (!input.reporterPubkey) throw new Error("Choose a Summary agent.");
  const host =
    lists.channels.length + lists.dms.length > 0
      ? await openDm({
          pubkeys: [input.reporterPubkey],
          expectedSignerPubkey: input.ownerPubkey,
        })
      : null;
  const workflowId = await syncSummarySchedule({
    ownerPubkey: input.ownerPubkey,
    previousWorkflowId: input.previousWorkflowId ?? null,
    hostChannelId: host?.id ?? null,
    hour: input.hour,
    minute: input.minute,
    timezone: input.timezone,
    reporterName: input.reporterName,
    schedule: input.schedule,
  });
  const payload: BriefConfigPayload = {
    v: 1,
    read_delegation: true,
    ...(input.schedule !== undefined ? { schedule: input.schedule } : {}),
    reporter_pubkey: input.reporterPubkey,
    hour: input.hour,
    minute: input.minute,
    timezone: input.timezone,
    auto_include: input.autoInclude,
    channels: lists.channels,
    dms: lists.dms,
    exclude: lists.exclude,
  };
  const tags: string[][] = [
    ["d", BRIEF_CONFIG_D_TAG],
    ["t", BRIEF_DISCOVERY_TAG],
    ["p", input.ownerPubkey],
    ["auto-include", input.autoInclude ? "true" : "false"],
    ["tz", input.timezone],
    ["hour", String(input.hour)],
    ["minute", String(input.minute)],
  ];
  if (
    input.reporterPubkey &&
    input.reporterPubkey.toLowerCase() !== input.ownerPubkey.toLowerCase()
  ) {
    tags.push(["p", input.reporterPubkey.toLowerCase()]);
  }
  if (workflowId) tags.push(["workflow", workflowId]);
  for (const id of lists.channels) tags.push(["include", "channel", id]);
  for (const id of lists.dms) tags.push(["include", "dm", id]);
  for (const id of lists.exclude) tags.push(["exclude", id]);

  const event = await signRelayEvent({
    allowSelfTagging: true,
    kind: KIND_BRIEF_CONFIG,
    createdAt: Math.max(
      Math.floor(Date.now() / 1000),
      (input.previousConfigCreatedAt ?? 0) + 1,
    ),
    content: JSON.stringify(payload),
    tags,
  });
  await relayClient.publishEvent(
    event,
    "Timed out publishing Summary sources",
    "Failed to save Summary sources",
  );
}

export async function reconcileReporterMembership(input: {
  reporterPubkey: string;
  previousChannels: readonly string[];
  nextChannels: readonly string[];
}): Promise<void> {
  const { added, removed } = membershipDiff(
    input.previousChannels,
    input.nextChannels,
  );
  const addFailures: string[] = [];
  for (const channelId of added) {
    const result = await addChannelMembers({
      channelId,
      pubkeys: [input.reporterPubkey],
      role: "bot",
    });
    const membershipError = result.errors.find(
      (error) =>
        error.pubkey.toLowerCase() === input.reporterPubkey.toLowerCase(),
    );
    if (membershipError) {
      addFailures.push(`${channelId}: ${membershipError.error}`);
    }
  }
  const removeFailures: string[] = [];
  for (const channelId of removed) {
    try {
      await removeChannelMember(channelId, input.reporterPubkey);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      removeFailures.push(`${channelId}: ${message}`);
    }
  }
  if (addFailures.length > 0 || removeFailures.length > 0) {
    const parts = [
      addFailures.length > 0
        ? `Could not add the reporter to: ${addFailures.join("; ")}`
        : null,
      removeFailures.length > 0
        ? `Could not revoke the reporter from: ${removeFailures.join("; ")}`
        : null,
    ].filter((part): part is string => part !== null);
    throw new Error(parts.join(" "));
  }
}
