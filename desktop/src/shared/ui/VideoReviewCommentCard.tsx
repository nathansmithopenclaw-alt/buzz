import { Check } from "lucide-react";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { UserAvatar } from "./UserAvatar";
import { VideoReviewTimecodeButton } from "./VideoReviewTimecodeButton";
import type { TimecodedComment, VideoReviewComment } from "./videoReviewTypes";

export function VideoReviewCommentCard({
  item,
  canReply,
  onReply,
  onSeek,
  onToggleReaction,
  replies,
}: {
  canReply: boolean;
  item: TimecodedComment;
  onReply: (item: TimecodedComment) => void;
  onSeek: (seconds: number) => void;
  onToggleReaction?: (
    comment: VideoReviewComment,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  replies: TimecodedComment[];
}) {
  return (
    <article className="rounded-lg bg-muted/40 p-3 text-sm text-foreground">
      <VideoReviewCommentBody
        canReply={canReply}
        item={item}
        onReply={onReply}
        onSeek={onSeek}
        onToggleReaction={onToggleReaction}
      />
      {replies.length > 0 ? (
        <div className="mt-3 space-y-3 border-l border-border pl-3">
          {replies.map((reply) => (
            <VideoReviewCommentBody
              canReply={canReply}
              item={reply}
              key={reply.comment.id}
              onReply={onReply}
              onSeek={onSeek}
              onToggleReaction={onToggleReaction}
            />
          ))}
        </div>
      ) : null}
    </article>
  );
}

function VideoReviewCommentBody({
  canReply,
  item,
  onReply,
  onSeek,
  onToggleReaction,
}: {
  canReply: boolean;
  item: TimecodedComment;
  onReply: (item: TimecodedComment) => void;
  onSeek: (seconds: number) => void;
  onToggleReaction?: (
    comment: VideoReviewComment,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
}) {
  const reactions = item.comment.reactions ?? [];
  const text = item.text || item.comment.body;
  const timecodeButton =
    item.seconds !== null && item.timecode ? (
      <VideoReviewTimecodeButton
        timecode={item.timecode}
        onClick={() => onSeek(item.seconds ?? 0)}
      />
    ) : null;

  return (
    <div>
      <div className="flex items-center gap-2">
        <UserAvatar
          avatarUrl={item.comment.avatarUrl ?? null}
          className="h-6 w-6 shadow-none"
          displayName={item.comment.author}
          shape={item.comment.isAgent ? "squircle" : "circle"}
          size="xs"
        />
        <p className="truncate text-sm font-semibold text-foreground">
          {item.comment.author}
        </p>
        <p className="shrink-0 text-xs text-muted-foreground">
          {item.comment.time}
        </p>
        {reactions.some((reaction) => reaction.reactedByCurrentUser) ? (
          <Check
            className="ml-auto h-4 w-4 shrink-0 text-primary"
            aria-hidden
          />
        ) : null}
      </div>
      <p className="mt-1.5 leading-5">
        {timecodeButton ? <>{timecodeButton} </> : null}
        {text}
      </p>
      {reactions.length > 0 ? (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {reactions.map((reaction) => (
            <button
              aria-label={`Toggle ${reaction.emoji} reaction`}
              aria-pressed={reaction.reactedByCurrentUser}
              className={cn(
                "flex h-7 items-center gap-1 rounded-full border px-2 text-xs transition-colors",
                reaction.reactedByCurrentUser
                  ? "border-primary/60 bg-primary/20 text-primary"
                  : "border-border bg-muted/40 text-muted-foreground hover:bg-muted",
              )}
              disabled={!onToggleReaction}
              key={reaction.emoji}
              type="button"
              onClick={() => {
                void onToggleReaction?.(
                  item.comment,
                  reaction.emoji,
                  Boolean(reaction.reactedByCurrentUser),
                );
              }}
            >
              {reaction.emojiUrl ? (
                <img
                  alt=""
                  className="h-4 w-4"
                  src={rewriteRelayUrl(reaction.emojiUrl)}
                />
              ) : (
                <span>{reaction.emoji}</span>
              )}
              <span>{reaction.count}</span>
            </button>
          ))}
        </div>
      ) : null}
      {canReply ? (
        <button
          className="mt-1.5 text-xs font-medium text-muted-foreground outline-hidden transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
          data-testid="video-review-comment-reply"
          type="button"
          onClick={() => onReply(item)}
        >
          Reply
        </button>
      ) : null}
    </div>
  );
}
