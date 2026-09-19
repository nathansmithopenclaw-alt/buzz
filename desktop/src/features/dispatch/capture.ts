import type { TimelineMessage } from "@/features/messages/types";
import type { RelayEvent } from "@/shared/api/types";

/** Only the leading command opts a new send into capture. */
export function parseTrackCommand(content: string): string | null {
  return content.startsWith("/track ") ? content.slice(7).trim() : null;
}

export function capturedMessage(event: RelayEvent): TimelineMessage {
  return {
    id: event.id,
    createdAt: event.created_at,
    pubkey: event.pubkey,
    signerPubkey: event.pubkey,
    author: "You",
    time: "",
    depth: 0,
    body: event.content,
    kind: event.kind,
    tags: event.tags,
    rootId: event.tags.find((tag) => tag[0] === "e" && tag[3] === "root")?.[1],
  };
}

export function newestOwnMessage(messages: TimelineMessage[], pubkey?: string) {
  if (!pubkey) return null;
  return messages
    .filter(
      (message) =>
        !message.pending && message.pubkey === pubkey && message.kind === 9,
    )
    .reduce<TimelineMessage | null>(
      (latest, message) =>
        !latest ||
        message.createdAt > latest.createdAt ||
        (message.createdAt === latest.createdAt && message.id > latest.id)
          ? message
          : latest,
      null,
    );
}
