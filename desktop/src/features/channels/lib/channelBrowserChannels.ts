import type { Channel } from "@/shared/api/types";
import {
  type ChannelSortMode,
  sortChannelsForSidebar,
} from "@/features/sidebar/lib/channelSortPreference";
import { scoreChannelMatch } from "./channelSearchScore";

export type ChannelBrowserTab = "all" | "joined" | "archived";
export type ChannelBrowserSort = ChannelSortMode | "members";

/** Project a normalized search query using the browser's visibility and ordering rules. */
export function channelBrowserChannels({
  channels,
  query,
  channelTypeFilter,
  tab,
  sort,
}: {
  channels: Channel[];
  query: string;
  channelTypeFilter?: "stream" | "forum";
  tab: ChannelBrowserTab;
  sort: ChannelBrowserSort;
}): Channel[] {
  const scores = new Map<string, number>();
  const visible = channels.filter((channel) => {
    if (
      channel.channelType === "dm" ||
      (channelTypeFilter && channel.channelType !== channelTypeFilter) ||
      (channel.archivedAt
        ? !channel.isMember
        : channel.visibility !== "open" && !channel.isMember)
    )
      return false;
    if (tab === "archived" && channel.archivedAt === null) return false;
    if (tab === "joined" && (channel.archivedAt !== null || !channel.isMember))
      return false;
    if (query.length === 0) return true;
    const score = scoreChannelMatch(channel, query);
    if (score === null) return false;
    scores.set(channel.id, score);
    return true;
  });
  const ordered =
    sort === "members"
      ? visible.sort(
          (a, b) =>
            b.memberCount - a.memberCount ||
            a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
        )
      : sortChannelsForSidebar(visible, sort);
  if (query.length === 0) return ordered;
  return ordered.sort(
    (a, b) =>
      (scores.get(a.id) ?? Number.POSITIVE_INFINITY) -
      (scores.get(b.id) ?? Number.POSITIVE_INFINITY),
  );
}
