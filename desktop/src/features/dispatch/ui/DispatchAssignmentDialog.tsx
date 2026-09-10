import { useDispatchJournalVersion } from "./useDispatchJournalVersion";
import { KeepAsDraftButton } from "./DispatchDraftsPanel";
import { keepAssignmentAsDraft } from "../taskAssignments";
import * as React from "react";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { Button } from "@/shared/ui/button";
import { Textarea } from "@/shared/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/shared/ui/dialog";
import { AssigneePickerDialog } from "./AssigneePickerDialog";
import { routingSuggestions, assignmentWarnings } from "../attention";
import {
  assignDispatchIssue,
  pendingAssignment,
  retryAssignment,
  type PendingAssignment,
} from "../taskAssignments";
import type { DispatchContext, DispatchIssue } from "../types";
export function DispatchAssignmentDialog({
  issue,
  context,
  viewer,
  refresh,
}: {
  issue: DispatchIssue;
  context: DispatchContext;
  viewer: string;
  refresh: () => Promise<DispatchContext>;
}) {
  const journalVersion = useDispatchJournalVersion();
  const [open, setOpen] = React.useState(false),
    [busy, setBusy] = React.useState(false),
    [base, setBase] = React.useState(issue),
    [executor, setExecutor] = React.useState<string | null>(null),
    [reviewer, setReviewer] = React.useState<string | null>(
      issue.assignee ?? viewer,
    ),
    [picker, setPicker] = React.useState<"executor" | "reviewer" | null>(null),
    [channel, setChannel] = React.useState(
      issue.homeChannelId ?? issue.streamId,
    ),
    [method, setMethod] = React.useState(
      "Check the resulting artifact against the accepted scope and record evidence.",
    ),
    [override, setOverride] = React.useState(""),
    [error, setError] = React.useState<string | null>(null),
    [message, setMessage] = React.useState<string | null>(null),
    [pending, setPending] = React.useState<PendingAssignment | null>(null);
  const { channels } = useChannelNavigation(),
    profiles = useUsersBatchQuery(
      [executor, reviewer].filter(Boolean) as string[],
    );
  const label = (key: string | null) =>
    key
      ? resolveUserLabel({ pubkey: key, profiles: profiles.data?.profiles })
      : "Choose a person";
  const suggestions = routingSuggestions(context, issue);
  const unfinished = context.linkedTasks.some(
    (task) =>
      task.issueId === issue.issueId &&
      !["failed", "verified"].includes(task.status),
  );
  const reasons = assignmentWarnings(context, issue, executor);
  // biome-ignore lint/correctness/useExhaustiveDependencies: Journal writes invalidate the durable data read by this effect.
  React.useEffect(() => {
    if (new Date(context.snapshotAt).valueOf() <= 0) return;
    let active = true;
    void pendingAssignment(issue.streamId, issue.issueId)
      .then((value) => {
        if (active) setPending(value);
      })
      .catch((error) => {
        if (active) setError(String(error));
      });
    return () => {
      active = false;
    };
  }, [issue.streamId, issue.issueId, context.snapshotAt, journalVersion]);
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!executor || !reviewer) return;
    setBusy(true);
    setError(null);
    try {
      await assignDispatchIssue({
        issue: base,
        executor,
        reviewer,
        channel,
        method,
        overrideReason: override,
        refresh,
      });
      setMessage(
        "Assignment confirmed in accepted task history. The issue owner is unchanged.",
      );
      setPending(null);
      setOpen(false);
    } catch (error) {
      setError(String(error));
      setPending(
        await pendingAssignment(issue.streamId, issue.issueId).catch(
          () => null,
        ),
      );
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="mt-3 space-y-2">
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={
            context.viewerRole === "none" ||
            Boolean(context.sectionErrors.tasks) ||
            unfinished ||
            ["done", "cancelled"].includes(issue.status) ||
            Boolean(pending)
          }
          onClick={() => {
            setBase(issue);
            setError(null);
            setOpen(true);
          }}
        >
          Assign execution
        </Button>
        {pending ? (
          <>
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => {
                setBusy(true);
                setError(null);
                void retryAssignment(pending, refresh)
                  .then(() => {
                    setPending(null);
                    setMessage("The saved assignment is confirmed.");
                  })
                  .catch((error) => setError(String(error)))
                  .finally(() => setBusy(false));
              }}
            >
              Check and resume saved assignment
            </Button>
            <KeepAsDraftButton
              disabled={busy}
              onKeep={async () => {
                await keepAssignmentAsDraft(pending);
                setPending(null);
              }}
            />
          </>
        ) : null}
      </div>
      {pending ? (
        <details className="text-xs text-muted-foreground">
          <summary>Saved assignment awaiting confirmation</summary>
          <p className="mt-1">
            Its exact signed event is retained. Resuming checks the accepted
            issue link before notifying the executor.
          </p>
          <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap">
            {pending.event.content}
          </pre>
        </details>
      ) : null}
      {unfinished ? (
        <p className="text-xs text-muted-foreground">
          Resolve the current attempt or its review before starting another.
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
      {message ? (
        <p role="status" className="text-sm">
          {message}
        </p>
      ) : null}
      <Dialog
        open={open}
        onOpenChange={(value) => {
          if (!busy) setOpen(value);
        }}
      >
        <DialogContent className="max-h-[90vh] max-w-2xl overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Assign execution</DialogTitle>
            <DialogDescription>
              Review who will do this work, who will independently check it, and
              where the accepted context will be shared.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={(event) => void submit(event)}>
            <fieldset disabled={busy} className="space-y-4">
              <p className="font-medium">{base.title}</p>
              <div className="grid gap-3 sm:grid-cols-2">
                <div>
                  <p className="mb-1 text-sm">Executor</p>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => setPicker("executor")}
                  >
                    {label(executor)}
                  </Button>
                </div>
                <div>
                  <p className="mb-1 text-sm">Independent reviewer</p>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => setPicker("reviewer")}
                  >
                    {label(reviewer)}
                  </Button>
                </div>
              </div>
              <AssigneePickerDialog
                open={picker !== null}
                onOpenChange={(value) => {
                  if (!value) setPicker(null);
                }}
                assignee={picker === "executor" ? executor : reviewer}
                disabled={busy}
                onAssign={(value) => {
                  if (picker === "executor") setExecutor(value);
                  else setReviewer(value);
                  setPicker(null);
                }}
              />
              {suggestions.length ? (
                <details>
                  <summary className="text-sm">Capacity suggestions</summary>
                  {suggestions.map((value) => (
                    <div
                      key={value.executor}
                      className="mt-2 rounded border border-border p-2 text-sm"
                    >
                      <button
                        type="button"
                        className="font-medium hover:underline"
                        onClick={() => setExecutor(value.executor)}
                      >
                        {label(value.executor)}
                      </button>
                      <p className="text-xs text-muted-foreground">
                        {value.explanation}
                      </p>
                    </div>
                  ))}
                </details>
              ) : null}
              <label
                htmlFor="dispatch-execution-channel"
                className="grid gap-1 text-sm"
              >
                Execution conversation
                <select
                  id="dispatch-execution-channel"
                  value={channel}
                  onChange={(event) => setChannel(event.target.value)}
                  className="h-9 rounded border border-input bg-background px-2"
                >
                  {!channels.some(
                    (channel) => channel.id === issue.streamId,
                  ) ? (
                    <option value={issue.streamId}>
                      Current Dispatch workspace
                    </option>
                  ) : null}
                  {channels.map((channel) => (
                    <option key={channel.id} value={channel.id}>
                      {channel.name}
                    </option>
                  ))}
                </select>
              </label>
              <label
                htmlFor="dispatch-verification-method"
                className="grid gap-1 text-sm"
              >
                How the result will be verified
                <Textarea
                  id="dispatch-verification-method"
                  required
                  maxLength={2048}
                  rows={3}
                  value={method}
                  onChange={(event) => setMethod(event.target.value)}
                />
              </label>
              {reasons.length ? (
                <div className="rounded border border-warning/40 bg-warning-bg p-3 text-sm">
                  <ul className="list-disc pl-4">
                    {reasons.map((reason) => (
                      <li key={reason}>{reason}</li>
                    ))}
                  </ul>
                  <label
                    htmlFor="dispatch-routing-override"
                    className="mt-3 grid gap-1"
                  >
                    Reason to proceed with this human override
                    <Textarea
                      id="dispatch-routing-override"
                      required
                      maxLength={2000}
                      value={override}
                      onChange={(event) => setOverride(event.target.value)}
                    />
                  </label>
                </div>
              ) : null}
              {base.tipEvent !== issue.tipEvent ? (
                <p role="status" className="text-sm text-warning">
                  The issue changed during this preview. Close and reopen to
                  review the latest scope.
                </p>
              ) : null}
              {executor === reviewer && executor ? (
                <p className="text-sm text-destructive">
                  Choose a different independent reviewer.
                </p>
              ) : null}
              {error ? (
                <p role="alert" className="text-sm text-destructive">
                  {error}
                </p>
              ) : null}
              <Button
                type="submit"
                disabled={
                  busy ||
                  !executor ||
                  !reviewer ||
                  executor === reviewer ||
                  !method.trim() ||
                  (reasons.length > 0 && !override.trim()) ||
                  base.tipEvent !== issue.tipEvent
                }
              >
                {busy
                  ? "Confirming assignment…"
                  : "Confirm assignment and notify executor"}
              </Button>
            </fieldset>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
