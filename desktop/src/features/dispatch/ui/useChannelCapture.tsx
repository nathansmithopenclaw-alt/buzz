import * as React from "react";
import type { TimelineMessage } from "@/features/messages/types";
import type { ChannelPaneProps } from "@/features/channels/ui/ChannelPane.types";
import { useFeatureEnabled } from "@/shared/features";
import {
  capturedMessage,
  newestOwnMessage,
  parseTrackCommand,
} from "../capture";

const ConversationDispatchDialog = React.lazy(async () => ({
  default: (await import("./ConversationDispatchDialog"))
    .ConversationDispatchDialog,
}));

/** Owns capture for this mounted channel; late sends cannot open in another channel. */
export function useChannelCapture(
  channel: string | null,
  messages: TimelineMessage[],
  pubkey?: string,
) {
  const enabled = useFeatureEnabled("dispatch");
  const [target, setTarget] = React.useState<{
    channel: string;
    message: TimelineMessage;
    title?: string;
  } | null>(null);
  const scope = React.useRef(channel);
  scope.current = channel;
  React.useEffect(() => {
    scope.current = channel;
    setTarget(null);
    return () => {
      scope.current = null;
    };
  }, [channel]);
  React.useEffect(() => {
    if (!enabled || !channel) return;
    const handleKey = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.repeat ||
        event.isComposing ||
        !event.metaKey ||
        !event.shiftKey ||
        event.ctrlKey ||
        event.altKey ||
        event.key.toLowerCase() !== "t" ||
        document.querySelector('[role="dialog"], [role="menu"]')
      )
        return;
      const message = newestOwnMessage(messages, pubkey);
      event.preventDefault();
      if (message) setTarget({ channel, message });
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [channel, enabled, messages, pubkey]);
  const send = React.useCallback(
    async (
      sender:
        | ChannelPaneProps["onSendMessage"]
        | ChannelPaneProps["onSendThreadReply"],
      ...args: Parameters<ChannelPaneProps["onSendMessage"]>
    ) => {
      const capturedChannel = args[3] ?? channel;
      const content = enabled ? parseTrackCommand(args[0]) : null;
      if (content !== null && !content)
        throw new Error("Write a message after /track.");
      const event = await sender(
        content ?? args[0],
        args[1],
        args[2],
        args[3],
        args[4],
        args[5],
      );
      if (
        content !== null &&
        event &&
        capturedChannel &&
        scope.current === channel
      ) {
        setTarget({
          channel: capturedChannel,
          message: capturedMessage(event),
          title: content.split("\n")[0].slice(0, 256),
        });
      }
    },
    [channel, enabled],
  );
  return {
    send,
    dialog:
      enabled && target && scope.current === channel ? (
        <React.Suspense fallback={null}>
          <ConversationDispatchDialog
            key={target.message.id}
            channel={target.channel}
            message={target.message}
            initialTitle={target.title}
            onClose={() => setTarget(null)}
          />
        </React.Suspense>
      ) : null,
  };
}
