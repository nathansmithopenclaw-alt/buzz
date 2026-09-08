import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
type VideoReviewReaction = {
  emoji: string;
  emojiUrl?: string;
  count: number;
  reactedByCurrentUser?: boolean;
  users: Array<{
    pubkey: string;
    displayName: string;
    avatarUrl: string | null;
  }>;
};
export type VideoReviewComment = {
  id: string;
  author: string;
  avatarUrl?: string | null;
  isAgent?: boolean;
  body: string;
  createdAt: number;
  time: string;
  /** Parent event id — when it points at another comment, this is a reply. */
  parentId?: string | null;
  reactions?: VideoReviewReaction[];
};
export type VideoReviewContext = {
  channelId?: string | null;
  channelName?: string;
  channelType?: ChannelType | null;
  comments: VideoReviewComment[];
  disabled?: boolean;
  isSending?: boolean;
  onSendComment?: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    /** Reply to this comment instead of the video message itself. */
    parentEventId?: string,
  ) => Promise<void>;
  onToggleCommentReaction?: (
    comment: VideoReviewComment,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  profiles?: UserProfileLookup;
  rootEventId?: string;
  title?: string;
};
export type TimecodedComment = {
  comment: VideoReviewComment;
  seconds: number | null;
  timecode: string | null;
  text: string;
};
