import { useDispatchJournalVersion } from "./useDispatchJournalVersion";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { useCommunities } from "@/features/communities/useCommunities";
import { KeepAsDraftButton } from "./DispatchDraftsPanel";
import { keepConversationUpdateAsDraft } from "../conversationUpdates";
import * as React from "react";
import { Button } from "@/shared/ui/button";
import { Textarea } from "@/shared/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/shared/ui/dialog";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { getRelayWsUrl } from "@/shared/api/tauri";
import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { buildDispatchLink } from "../dispatchLink";
import {
  conversationTargets,
  conversationUpdateChoices,
  pendingConversationUpdate,
  retryConversationUpdate,
  shareConversationUpdate,
  type PendingConversationUpdate,
  type UpdateChoice,
} from "../conversationUpdates";
import type { DispatchContext, DispatchIssue } from "../types";
import { DispatchReferenceLink } from "./DispatchReferenceLink";
export function ConversationUpdatePanel({
  issue,
  context,
  refresh,
}: {
  issue: DispatchIssue;
  context: DispatchContext;
  refresh: () => Promise<DispatchContext>;
}) {
  const journalVersion = useDispatchJournalVersion();
  const { channels } = useChannelNavigation();
  const { activeCommunity } = useCommunities();
  const scopeRef = React.useRef(activeCommunity?.relayUrl);
  scopeRef.current = activeCommunity?.relayUrl;
  const [draft, setDraft] = React.useState<{
      choice: UpdateChoice;
      scope: { relay: string; signer: string };
      body: string;
      target: { channel: string; source: string | null };
    } | null>(null),
    [pending, setPending] = React.useState<PendingConversationUpdate | null>(
      null,
    ),
    [busy, setBusy] = React.useState(false),
    [error, setError] = React.useState<string | null>(null),
    [sent, setSent] = React.useState<string | null>(null),
    [notify, setNotify] = React.useState(true);
  const id = React.useId();
  // biome-ignore lint/correctness/useExhaustiveDependencies: Journal writes invalidate the durable data read by this effect.
  React.useEffect(() => {
    if (!context.snapshotAt) return;
    let current = true;
    void pendingConversationUpdate(issue.streamId, issue.issueId)
      .then((value) => {
        if (current) setPending(value);
      })
      .catch((error) => {
        if (current) setError(String(error));
      });
    return () => {
      current = false;
    };
  }, [issue.streamId, issue.issueId, context.snapshotAt, journalVersion]);
  const targets = conversationTargets(issue),
    choices = conversationUpdateChoices(context, issue);
  const open = async (choice: UpdateChoice, target = targets[0]) => {
    if (!target) return;
    setError(null);
    try {
      const displayedRelay = scopeRef.current;
      const [relay, identity] = await Promise.all([
        getRelayWsUrl(),
        getIdentity(),
      ]);
      if (
        displayedRelay !== scopeRef.current ||
        displayedRelay?.replace(/\/$/, "") !== relay.replace(/\/$/, "")
      )
        throw new Error(
          "Community changed. Reopen this issue in its original community.",
        );
      setDraft({
        scope: { relay, signer: identity.pubkey },
        choice,
        target,
        body: `[${issue.title.replaceAll("[", "").replaceAll("]", "")} ](${buildDispatchLink({ workspace: issue.streamId, issue: issue.issueId, relay })})\n\n${choice.body}`,
      });
    } catch (error) {
      setError(String(error));
    }
  };
  const finish = async (
    operation: () => Promise<import("@/shared/api/types").RelayEvent>,
  ) => {
    setBusy(true);
    setError(null);
    try {
      const event = await operation();
      setSent(
        buildMessageLink({
          channelId: event.tags.find((tag) => tag[0] === "h")?.[1] ?? "",
          messageId: event.id,
        }),
      );
      setDraft(null);
      setPending(null);
    } catch (error) {
      setError(String(error));
      void pendingConversationUpdate(issue.streamId, issue.issueId)
        .then(setPending)
        .catch((recovery) => setError(`${String(error)} ${String(recovery)}`));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section
      className="mt-5 border-t border-border pt-5"
      aria-label="Conversation updates"
    >
      <h2 className="text-sm font-semibold">Keep the conversation informed</h2>
      <p className="mt-1 text-xs text-muted-foreground">
        Share a specific status change, blocker, result or decision. Each change
        is sent at most once to the selected source.
      </p>
      <Button
        className="mt-3"
        size="sm"
        variant="outline"
        disabled={
          busy ||
          Boolean(pending) ||
          !targets.length ||
          context.viewerRole === "none"
        }
        onClick={() => void open(choices[0])}
      >
        Share an update
      </Button>
      {!targets.length ? (
        <p className="mt-2 text-xs text-muted-foreground">
          Add a conversation link or home channel to share an update.
        </p>
      ) : null}
      {pending ? (
        <div className="mt-3 rounded border border-warning/40 p-3 text-sm">
          <p>An update is saved for confirmation.</p>
          <details className="my-2">
            <summary>Review saved message</summary>
            <p className="mt-2 whitespace-pre-wrap break-words">
              {pending.event.content}
            </p>
          </details>
          <Button
            size="sm"
            disabled={busy}
            onClick={() =>
              void finish(() => retryConversationUpdate(pending, refresh))
            }
          >
            Check and retry update
          </Button>
          <KeepAsDraftButton
            disabled={busy}
            onKeep={async () => {
              await keepConversationUpdateAsDraft(pending);
              setPending(null);
            }}
          />
        </div>
      ) : null}
      {sent ? (
        <div className="mt-2 text-sm">
          <span>Shared in the conversation. </span>
          <DispatchReferenceLink value={{ type: "conversation", ref: sent }} />
        </div>
      ) : null}
      {!draft && error ? (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}
      <Dialog
        open={Boolean(draft)}
        onOpenChange={(open) => {
          if (!open && !busy) setDraft(null);
        }}
      >
        <DialogContent className="max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Share a work update</DialogTitle>
            <DialogDescription>
              Review the message and destination. Current permissions and the
              referenced change are checked again before sending.
            </DialogDescription>
          </DialogHeader>
          <label className="grid gap-2 text-sm">
            What changed?
            <select
              className="h-9 rounded border bg-background px-2"
              value={
                draft
                  ? `${draft.choice.purpose}:${draft.choice.changeEvent}`
                  : ""
              }
              disabled={busy || Boolean(pending)}
              onChange={(event) => {
                const choice = choices.find(
                  (choice) =>
                    `${choice.purpose}:${choice.changeEvent}` ===
                    event.target.value,
                );
                if (choice) void open(choice, draft?.target);
              }}
            >
              {choices.map((choice) => (
                <option
                  key={`${choice.purpose}:${choice.changeEvent}`}
                  value={`${choice.purpose}:${choice.changeEvent}`}
                >
                  {choice.label}
                </option>
              ))}
            </select>
          </label>
          <label className="grid gap-2 text-sm">
            Conversation
            <select
              className="h-9 rounded border bg-background px-2"
              value={
                draft
                  ? `${draft.target.channel}:${draft.target.source ?? ""}`
                  : ""
              }
              disabled={busy || Boolean(pending)}
              onChange={(event) => {
                const target = targets.find(
                  (target) =>
                    `${target.channel}:${target.source ?? ""}` ===
                    event.target.value,
                );
                if (draft && target) setDraft({ ...draft, target });
              }}
            >
              {targets.map((target, index) => (
                <option
                  key={`${target.channel}:${target.source ?? ""}`}
                  value={`${target.channel}:${target.source ?? ""}`}
                >
                  {channels.find((channel) => channel.id === target.channel)
                    ?.name ?? `Linked conversation ${index + 1}`}
                  {target.source ? " · reply to source" : " · channel update"}
                </option>
              ))}
            </select>
          </label>
          <label htmlFor={id} className="grid gap-2 text-sm">
            Message
            <Textarea
              id={id}
              rows={8}
              value={draft?.body ?? ""}
              maxLength={14000}
              disabled={busy || Boolean(pending)}
              onChange={(event) => {
                if (draft) setDraft({ ...draft, body: event.target.value });
              }}
            />
          </label>
          {draft?.choice.recipient ? (
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={notify}
                disabled={busy}
                onChange={(event) => setNotify(event.target.checked)}
              />
              Notify the named decider
            </label>
          ) : null}
          {error ? (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          ) : null}
          <Button
            disabled={busy || Boolean(pending) || !draft?.body.trim()}
            onClick={() => {
              if (draft)
                void finish(() =>
                  shareConversationUpdate({
                    expectedScope: draft.scope,
                    notice: {
                      workspace: issue.streamId,
                      issue: issue.issueId,
                      source_event: draft.target.source,
                      change_event: draft.choice.changeEvent,
                      purpose: draft.choice.purpose,
                    },
                    channel: draft.target.channel,
                    body: draft.body,
                    recipient: notify ? draft.choice.recipient : undefined,
                    refresh,
                  }),
                );
            }}
          >
            {busy ? "Confirming…" : "Share reviewed update"}
          </Button>
        </DialogContent>
      </Dialog>
    </section>
  );
}
