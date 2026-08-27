export type SummaryConversationKind = "channel" | "dm";

export type SummaryConversation = {
  id: string;
  name: string;
  kind: SummaryConversationKind;
  archived: boolean;
};

export type SummarySourceConfig = {
  autoInclude: boolean;
  include: string[];
  exclude: string[];
};

export function isConversationChecked(
  config: SummarySourceConfig,
  conversation: SummaryConversation,
): boolean {
  if (conversation.archived) return false;
  if (config.exclude.includes(conversation.id)) return false;
  if (config.include.includes(conversation.id)) return true;
  return config.autoInclude;
}

export function toggleConversation(
  config: SummarySourceConfig,
  conversation: SummaryConversation,
  checked: boolean,
): SummarySourceConfig {
  const id = conversation.id;
  const include = config.include.filter((value) => value !== id);
  const exclude = config.exclude.filter((value) => value !== id);
  if (checked) {
    include.push(id);
  } else {
    exclude.push(id);
  }
  return { ...config, include, exclude };
}

export function partitionLiveConversations(
  conversations: SummaryConversation[],
): {
  channels: SummaryConversation[];
  dms: SummaryConversation[];
} {
  const live = conversations.filter((conversation) => !conversation.archived);
  return {
    channels: live.filter((conversation) => conversation.kind === "channel"),
    dms: live.filter((conversation) => conversation.kind === "dm"),
  };
}

export function listsFromChecked(
  conversations: SummaryConversation[],
  config: SummarySourceConfig,
): { channels: string[]; dms: string[]; exclude: string[] } {
  const channels: string[] = [];
  const dms: string[] = [];
  for (const conversation of conversations) {
    if (conversation.archived) continue;
    if (!isConversationChecked(config, conversation)) continue;
    if (conversation.kind === "dm") dms.push(conversation.id);
    else channels.push(conversation.id);
  }
  return {
    channels,
    dms,
    exclude: [...config.exclude],
  };
}

export function defaultSourceConfig(
  conversations: SummaryConversation[],
): SummarySourceConfig {
  return {
    autoInclude: true,
    include: conversations
      .filter((conversation) => !conversation.archived)
      .map((conversation) => conversation.id),
    exclude: [],
  };
}

export function formatLocalTime(hour: number, minute: number): string {
  return `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`;
}

export function parseLocalTime(
  value: string,
): { hour: number; minute: number } | null {
  const match = /^(\d{2}):(\d{2})$/.exec(value);
  if (!match) return null;
  const hour = Number(match[1]);
  const minute = Number(match[2]);
  if (hour > 23 || minute > 59) return null;
  return { hour, minute };
}
