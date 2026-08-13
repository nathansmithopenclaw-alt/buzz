import * as React from "react";

import { useChannelMembersQuery } from "@/features/channels/hooks";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { publishDispatchIssueCreate } from "../mutations";
import type {
  DispatchContext,
  DispatchPriority,
  DispatchProject,
  DispatchStatus,
} from "../types";

const fieldClass =
  "h-9 rounded-lg border border-input/50 bg-background px-2 text-sm text-foreground";

export function NewIssueDialog({
  open,
  onOpenChange,
  streamId,
  identifierPrefix,
  projects,
  refresh,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  streamId: string;
  identifierPrefix: string;
  projects: DispatchProject[];
  refresh: () => Promise<DispatchContext>;
}) {
  const members = useChannelMembersQuery(streamId, open);
  const [requestId, setRequestId] = React.useState(() => crypto.randomUUID());
  const [title, setTitle] = React.useState("");
  const [body, setBody] = React.useState("");
  const [status, setStatus] = React.useState<DispatchStatus>("triage");
  const [priority, setPriority] = React.useState<DispatchPriority | "">("");
  const [projectId, setProjectId] = React.useState("");
  const [labels, setLabels] = React.useState("");
  const [assignee, setAssignee] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [submitting, setSubmitting] = React.useState(false);

  const reset = () => {
    setRequestId(crypto.randomUUID());
    setTitle("");
    setBody("");
    setStatus("triage");
    setPriority("");
    setProjectId("");
    setLabels("");
    setAssignee("");
    setError(null);
  };
  const changeOpen = (next: boolean) => {
    if (submitting) return;
    if (!next) reset();
    onOpenChange(next);
  };
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      await publishDispatchIssueCreate({
        input: {
          streamId,
          requestId,
          identifier: `${identifierPrefix}-${requestId.replaceAll("-", "")}`,
          title,
          body,
          status,
          priority: priority || null,
          projectId: projectId || null,
          labels: labels.split(","),
          assignee: assignee || null,
        },
        refresh,
      });
      reset();
      onOpenChange(false);
    } catch (value) {
      setError(value instanceof Error ? value.message : String(value));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog onOpenChange={changeOpen} open={open}>
      <DialogContent data-testid="dispatch-new-issue-dialog">
        <DialogHeader>
          <DialogTitle>New issue</DialogTitle>
          <DialogDescription>
            Create a revision-zero Tracker issue in this workspace.
          </DialogDescription>
        </DialogHeader>
        <form className="grid gap-4" onSubmit={submit}>
          <label
            className="grid gap-1 text-xs font-medium"
            htmlFor="dispatch-new-issue-title"
          >
            Title
            <Input
              autoFocus
              data-testid="dispatch-new-issue-title"
              id="dispatch-new-issue-title"
              maxLength={240}
              onChange={(event) => setTitle(event.target.value)}
              required
              value={title}
            />
          </label>
          <label
            className="grid gap-1 text-xs font-medium"
            htmlFor="dispatch-new-issue-body"
          >
            Body
            <Textarea
              data-testid="dispatch-new-issue-body"
              id="dispatch-new-issue-body"
              onChange={(event) => setBody(event.target.value)}
              rows={5}
              value={body}
            />
          </label>
          <div className="grid grid-cols-2 gap-3">
            <label className="grid gap-1 text-xs font-medium">
              Status
              <select
                className={fieldClass}
                onChange={(event) =>
                  setStatus(event.target.value as DispatchStatus)
                }
                value={status}
              >
                {(
                  [
                    "triage",
                    "backlog",
                    "todo",
                    "in_progress",
                    "in_review",
                    "done",
                    "cancelled",
                  ] as const
                ).map((value) => (
                  <option key={value} value={value}>
                    {value.replaceAll("_", " ")}
                  </option>
                ))}
              </select>
            </label>
            <label className="grid gap-1 text-xs font-medium">
              Priority
              <select
                className={fieldClass}
                data-testid="dispatch-new-issue-priority"
                onChange={(event) =>
                  setPriority(event.target.value as DispatchPriority | "")
                }
                value={priority}
              >
                <option value="">No priority</option>
                {(["P0", "P1", "P2", "P3"] as const).map((value) => (
                  <option key={value}>{value}</option>
                ))}
              </select>
            </label>
            <label className="grid gap-1 text-xs font-medium">
              Project
              <select
                className={fieldClass}
                data-testid="dispatch-new-issue-project"
                onChange={(event) => setProjectId(event.target.value)}
                value={projectId}
              >
                <option value="">No project</option>
                {projects.map((project) => (
                  <option key={project.projectId} value={project.projectId}>
                    {project.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="grid gap-1 text-xs font-medium">
              Assignee
              <select
                className={fieldClass}
                onChange={(event) => setAssignee(event.target.value)}
                value={assignee}
              >
                <option value="">Unassigned</option>
                {(members.data ?? []).map((member) => (
                  <option key={member.pubkey} value={member.pubkey}>
                    {member.displayName ?? truncatePubkey(member.pubkey)}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <label
            className="grid gap-1 text-xs font-medium"
            htmlFor="dispatch-new-issue-labels"
          >
            Labels{" "}
            <span className="font-normal text-muted-foreground">
              Comma-separated
            </span>
            <Input
              data-testid="dispatch-new-issue-labels"
              id="dispatch-new-issue-labels"
              onChange={(event) => setLabels(event.target.value)}
              value={labels}
            />
          </label>
          {error ? (
            <p className="text-xs text-destructive" role="alert">
              {error}
            </p>
          ) : null}
          <DialogFooter>
            <Button
              disabled={submitting}
              onClick={() => changeOpen(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              data-testid="dispatch-new-issue-submit"
              disabled={submitting || !title.trim()}
              type="submit"
            >
              {submitting ? "Creating…" : "Create issue"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
