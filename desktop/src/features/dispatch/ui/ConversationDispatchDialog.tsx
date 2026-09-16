import { invokeTauri } from "@/shared/api/tauri";
import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { useCommunities } from "@/features/communities/useCommunities";
import type { TimelineMessage } from "@/features/messages/types";
import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { useDispatchContext, useDispatchWorkspaces } from "../hooks";
import { assertDispatchAudience } from "../audience";
import {
  publishDispatchIssueCreate,
  publishDispatchIssueMutation,
} from "../mutations";
import { issueAlias } from "../issueEditing";
import type { DispatchIssue } from "../types";

export function ConversationDispatchDialog({
  message,
  channel,
  onClose,
}: {
  message: TimelineMessage;
  channel: string;
  onClose: () => void;
}) {
  const navigate = useNavigate(),
    { activeCommunity } = useCommunities();
  const scope = React.useRef(activeCommunity?.relayUrl);
  const currentScope = React.useRef(activeCommunity?.relayUrl);
  currentScope.current = activeCommunity?.relayUrl;
  const workspaces = useDispatchWorkspaces();
  const [workspace, setWorkspace] = React.useState<string | null>(null);
  const context = useDispatchContext(workspace);
  const [mode, setMode] = React.useState<"create" | "attach">("create"),
    [title, setTitle] = React.useState(""),
    [body, setBody] = React.useState(""),
    [query, setQuery] = React.useState("");
  const [copySource, setCopySource] = React.useState(false),
    [busy, setBusy] = React.useState(false),
    [error, setError] = React.useState<string | null>(null),
    [saved, setSaved] = React.useState<DispatchIssue | null>(null);
  React.useEffect(() => {
    if (!workspace && !workspaces.isLoading) {
      const populated = workspaces.workspaces.filter(
        (value) => value.trackerEventCount > 0,
      );
      const initial =
        workspaces.workspaces.find((value) => value.id === channel) ??
        (populated.length === 1 ? populated[0] : null);
      if (initial) setWorkspace(initial.id);
    }
  }, [workspace, workspaces.workspaces, workspaces.isLoading, channel]);
  const sourceLink = buildMessageLink({
    channelId: channel,
    messageId: message.id,
    threadRootId: message.rootId,
  });
  const linked = context.issues.filter(
    (issue) =>
      issue.sourceEventId === message.id ||
      issue.externalRefs.some(
        (ref) =>
          ref.ref === sourceLink ||
          (ref.type === "source_event" && ref.ref === message.id),
      ),
  );
  const ensureScope = () => {
    if (scope.current !== currentScope.current)
      throw new Error(
        "Return to the original community before saving this conversation.",
      );
  };
  const create = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!workspace) return;
    setBusy(true);
    setError(null);
    try {
      ensureScope();
      const requestId = await invokeTauri<string>(
        "dispatch_capture_request_id",
        { sourceEvent: message.id },
      );
      const fresh = await context.refresh();
      ensureScope();
      const existing = fresh.issues.find(
        (issue) =>
          issue.issueId === requestId ||
          issue.sourceEventId === message.id ||
          issue.externalRefs.some((ref) => ref.ref === sourceLink),
      );
      if (existing) {
        setSaved(existing);
        return;
      }
      if (copySource) await assertDispatchAudience(channel, workspace);
      ensureScope();
      const result = await publishDispatchIssueCreate({
        input: {
          streamId: workspace,
          requestId,
          identifier: `D-${requestId.replaceAll("-", "")}`,
          title,
          body: copySource
            ? `${body}\n\nSource excerpt (${new Date(message.createdAt * 1000).toLocaleString()}):\n\n${message.body.slice(0, 16000)}`
            : body,
          status: "triage",
          priority: null,
          projectId: null,
          labels: [],
          assignee: null,
          homeChannelId: channel,
          sourceEventId: message.id,
          externalRefs: [{ type: "conversation", ref: sourceLink }],
        },
        refresh: context.refresh,
      });
      setSaved(result.issue);
    } catch (error) {
      setError(String(error));
      // Another client can win the source capture while this one is publishing.
      // Show that accepted issue; retain the locally entered text for review.
      try {
        const current = await context.refresh();
        ensureScope();
        const existing = current.issues.find(
          (issue) =>
            issue.sourceEventId === message.id ||
            issue.externalRefs.some((ref) => ref.ref === sourceLink),
        );
        if (existing) setSaved(existing);
      } catch {
        /* Original error and draft remain visible. */
      }
    } finally {
      setBusy(false);
    }
  };
  const attach = async (issue: DispatchIssue) => {
    setBusy(true);
    setError(null);
    try {
      ensureScope();
      const updated = await publishDispatchIssueMutation({
        issue,
        mutation: {
          type: "edit",
          expectedTip: issue.tipEvent,
          fields: {
            externalRefs: [
              ...issue.externalRefs,
              { type: "conversation", ref: sourceLink },
            ],
          },
          reason: "Linked an existing conversation",
        },
        viewerRole: context.viewerRole,
        onOptimistic: () => {},
        onRollback: () => {},
        refresh: context.refresh,
      });
      setSaved(updated);
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  const open = (issue: DispatchIssue) => {
    onClose();
    void navigate({
      to: "/dispatch/issue/$issueId",
      params: { issueId: issue.issueId },
      search: { workspace: issue.streamId },
    });
  };
  return (
    <Dialog
      open
      onOpenChange={(value) => {
        if (!value && !busy) onClose();
      }}
    >
      <DialogContent
        className="max-h-[85vh] max-w-2xl overflow-y-auto"
        data-testid="conversation-dispatch-dialog"
      >
        <DialogHeader>
          <DialogTitle>Track this conversation</DialogTitle>
          <DialogDescription>
            Create a clear next step or connect this message to work already in
            Dispatch.
          </DialogDescription>
        </DialogHeader>
        <label className="grid gap-1 text-sm">
          Workspace
          <select
            value={workspace ?? ""}
            disabled={busy || Boolean(saved)}
            className="h-9 rounded border border-input bg-background px-2"
            onChange={(event) => {
              setWorkspace(event.target.value || null);
              setCopySource(false);
            }}
          >
            <option value="">Choose a workspace</option>
            {workspaces.workspaces.map((value) => (
              <option key={value.id} value={value.id}>
                {value.name}
              </option>
            ))}
          </select>
        </label>
        {workspaces.error || context.error ? (
          <p role="alert" className="text-sm text-destructive">
            {String(workspaces.error ?? context.error)}
          </p>
        ) : null}
        {saved ? (
          <div role="status" className="rounded border border-border p-3">
            <p className="text-sm">
              Linked to {issueAlias(saved, context.issues)} · {saved.title}
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              Current status: {saved.status.replaceAll("_", " ")}
            </p>
            <Button className="mt-3" onClick={() => open(saved)}>
              Open issue
            </Button>
          </div>
        ) : (
          <>
            {linked.length ? (
              <section className="rounded border border-border p-3">
                <h3 className="text-sm font-medium">Already linked</h3>
                {linked.map((issue) => (
                  <Button
                    key={issue.issueId}
                    variant="link"
                    className="h-auto whitespace-normal text-left"
                    onClick={() => open(issue)}
                  >
                    {issueAlias(issue, context.issues)} · {issue.title} ·{" "}
                    {issue.status.replaceAll("_", " ")}
                  </Button>
                ))}
              </section>
            ) : null}
            <div className="flex gap-2">
              <Button
                variant={mode === "create" ? "default" : "outline"}
                size="sm"
                onClick={() => setMode("create")}
              >
                Create issue
              </Button>
              <Button
                variant={mode === "attach" ? "default" : "outline"}
                size="sm"
                onClick={() => setMode("attach")}
              >
                Attach to existing issue
              </Button>
            </div>
            {mode === "create" ? (
              <form onSubmit={(event) => void create(event)}>
                <fieldset
                  disabled={busy || !workspace || context.isLoading}
                  className="space-y-3"
                >
                  <label htmlFor="capture-title" className="grid gap-1 text-sm">
                    Outcome or next step
                    <Input
                      id="capture-title"
                      value={title}
                      required
                      maxLength={256}
                      onChange={(event) => setTitle(event.target.value)}
                    />
                  </label>
                  <label htmlFor="capture-body" className="grid gap-1 text-sm">
                    Context for the workspace
                    <Textarea
                      id="capture-body"
                      value={body}
                      rows={4}
                      maxLength={16000}
                      onChange={(event) => setBody(event.target.value)}
                    />
                  </label>
                  <label className="flex items-start gap-2 text-sm">
                    <input
                      type="checkbox"
                      checked={copySource}
                      onChange={(event) => setCopySource(event.target.checked)}
                    />
                    Include the message text after checking both conversations’
                    permissions
                  </label>
                  <p className="text-xs text-muted-foreground">
                    A link to the original message is always included. Copying
                    its text is allowed only when everyone in the destination
                    can read the source.
                  </p>
                  <Button
                    type="submit"
                    disabled={
                      !title.trim() ||
                      linked.length > 0 ||
                      context.viewerRole === "none"
                    }
                  >
                    Create linked issue
                  </Button>
                </fieldset>
              </form>
            ) : (
              <section>
                <Input
                  aria-label="Find an existing issue"
                  placeholder="Search title or identifier"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                />
                <div className="mt-2 max-h-72 overflow-y-auto">
                  {context.issues
                    .filter((issue) =>
                      `${issue.identifier} ${issue.title}`
                        .toLowerCase()
                        .includes(query.toLowerCase()),
                    )
                    .slice(0, 30)
                    .map((issue) => (
                      <div
                        key={issue.issueId}
                        className="flex items-center justify-between gap-2 border-b border-border py-2"
                      >
                        <span className="text-sm">
                          {issueAlias(issue, context.issues)} · {issue.title}
                        </span>
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={
                            busy ||
                            context.viewerRole === "none" ||
                            linked.some(
                              (value) => value.issueId === issue.issueId,
                            )
                          }
                          onClick={() => void attach(issue)}
                        >
                          Attach
                        </Button>
                      </div>
                    ))}
                </div>
              </section>
            )}
          </>
        )}
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
