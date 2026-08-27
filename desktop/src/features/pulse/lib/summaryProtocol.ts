import { relayClient } from "@/shared/api/relayClient";
import { addChannelMembers, signRelayEvent } from "@/shared/api/tauri";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { KIND_BRIEF_CONFIG, KIND_BRIEF_REPORT } from "@/shared/constants/kinds";

import type {
  SummaryConversation,
  SummarySourceConfig,
} from "./summarySources";
import { defaultSourceConfig, listsFromChecked } from "./summarySources";

export const BRIEF_DISCOVERY_TAG = "buzz-brief";
export const BRIEF_CONFIG_D_TAG = "summary";

export type BriefConfigPayload = {
  v: 1;
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
): SummaryConversation[] {
  return channels
    .filter((channel) => channel.isMember)
    .map((channel) => ({
      id: channel.id,
      name:
        channel.channelType === "dm"
          ? channel.name || "Direct message"
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

export function reportFromEvent(event: RelayEvent): SummaryReport | null {
  try {
    const parsed = JSON.parse(event.content) as BriefReportPayload;
    if (parsed.v !== 1 || tagValue(event, "t") !== BRIEF_DISCOVERY_TAG) {
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

export async function fetchBriefConfig(
  pubkey: string,
): Promise<RelayEvent | null> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_BRIEF_CONFIG],
    authors: [pubkey],
    "#d": [BRIEF_CONFIG_D_TAG],
    limit: 1,
  });
  return events[0] ?? null;
}

export async function fetchBriefReports(
  pubkey: string,
): Promise<SummaryReport[]> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_BRIEF_REPORT],
    "#p": [pubkey],
    "#t": [BRIEF_DISCOVERY_TAG],
    limit: 50,
  });
  return events
    .map(reportFromEvent)
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
  conversations: SummaryConversation[];
  sourceConfig: SummarySourceConfig;
}): Promise<void> {
  const lists = listsFromChecked(input.conversations, input.sourceConfig);
  const payload: BriefConfigPayload = {
    v: 1,
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
  for (const id of lists.channels) tags.push(["include", "channel", id]);
  for (const id of lists.dms) tags.push(["include", "dm", id]);
  for (const id of lists.exclude) tags.push(["exclude", id]);

  const event = await signRelayEvent({
    kind: KIND_BRIEF_CONFIG,
    content: JSON.stringify(payload),
    tags,
  });
  await relayClient.publishEvent(
    event,
    "Timed out publishing Summary sources",
    "Failed to save Summary sources",
  );

  if (input.reporterPubkey) {
    await Promise.all(
      lists.channels.map((channelId) =>
        addChannelMembers({
          channelId,
          pubkeys: [input.reporterPubkey as string],
          role: "bot",
        }).catch((error) => {
          console.warn("Could not add reporter to channel", channelId, error);
        }),
      ),
    );
  }
}
