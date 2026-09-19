import { useDispatchJournalVersion } from "./useDispatchJournalVersion";
import { KeepAsDraftButton } from "./DispatchDraftsPanel";
import { keepAssignmentAsDraft } from "../taskAssignments";
import * as React from "react";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { activeRecords } from "../recordTypes";
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
import { assignmentWarnings } from "../attention";
import {
  assignDispatchIssue,
  pendingAssignment,
  retryAssignment,
  type PendingAssignment,
} from "../taskAssignments";
import type { DispatchContext, DispatchIssue } from "../types";
/** The issue owner reviews by default, unless they will execute the work. */
export function defaultReviewer(owner: string | null, executor: string | null) {
  return owner && owner !== executor ? owner : null;
}

export function DispatchAssignmentDialog({
  issue,
  context,
  refresh,
}: {
  issue: DispatchIssue;
  context: DispatchContext;
  refresh: () => Promise<DispatchContext>;
}) {
  const journalVersion = useDispatchJournalVersion();
  const [open, setOpen] = React.useState(false),
    [busy, setBusy] = React.useState(false),
    [base, setBase] = React.useState(issue),
    [executor, setExecutor] = React.useState<string | null>(null),
    [reviewer, setReviewer] = React.useState<string | null>(
      defaultReviewer(issue.assignee, null),
    ),
    [picker, setPicker] = React.useState<"executor" | "reviewer" | null>(null),
    [notes, setNotes] = React.useState(""),
    [deadline, setDeadline] = React.useState(""),
    [advanced, setAdvanced] = React.useState(false),
    [override, setOverride] = React.useState(""),
    [error, setError] = React.useState<string | null>(null),
    [message, setMessage] = React.useState<string | null>(null),
    [pending, setPending] = React.useState<PendingAssignment | null>(null);
  const profiles = useUsersBatchQuery(
    [executor, reviewer].filter(Boolean) as string[],
  );
  const label = (key: string | null) =>
    key
      ? resolveUserLabel({ pubkey: key, profiles: profiles.data?.profiles })
      : "Choose a person";
  const capacityExecutors = React.useMemo(
    () =>
      activeRecords(context.records, "capacity").map(
        (entry) => entry.record.payload.executor,
      ),
    [context.records],
  );
  const selectExecutor = (value: string | null) => {
    setExecutor(value);
    setReviewer((current) =>
      current === executor || current === value || !current
        ? defaultReviewer(base.assignee, value)
        : current,
    );
  };
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
        channel: base.homeChannelId ?? base.streamId,
        method:
          "Check the resulting artifact against the accepted scope and record evidence.",
        notes,
        deadline: deadline ? new Date(deadline).toISOString() : undefined,
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
    <div className="space-y-2">
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          disabled={
            context.viewerRole === "none" ||
            Boolean(context.sectionErrors.tasks) ||
            unfinished ||
            ["done", "cancelled"].includes(issue.status) ||
            Boolean(pending)
          }
          onClick={() => {
            setBase(issue);
            setExecutor(null);
            setReviewer(defaultReviewer(issue.assignee, null));
            setNotes("");
            setDeadline("");
            setAdvanced(false);
            setOverride("");
            setError(null);
            setOpen(true);
          }}
        >
          Assign
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
            <DialogTitle>Assign</DialogTitle>
            <DialogDescription>
              Choose who will do the work and who will review it. The assignment
              goes to the issue’s home channel.
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
                    aria-label="Executor"
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
                    aria-label="Independent reviewer"
                    aria-describedby="dispatch-reviewer-help"
                    onClick={() => setPicker("reviewer")}
                  >
                    {label(reviewer)}
                  </Button>
                  <p
                    id="dispatch-reviewer-help"
                    className="mt-1 text-xs text-muted-foreground"
                  >
                    Reviews the result. Cannot be the executor.
                  </p>
                </div>
              </div>
              <AssigneePickerDialog
                open={picker !== null}
                onOpenChange={(value) => {
                  if (!value) setPicker(null);
                }}
                assignee={picker === "executor" ? executor : reviewer}
                disabled={busy}
                capacityPubkeys={
                  picker === "executor" ? capacityExecutors : undefined
                }
                excludedPubkey={picker === "reviewer" ? executor : null}
                onAssign={(value) => {
                  if (picker === "executor") selectExecutor(value);
                  else setReviewer(value);
                  setPicker(null);
                }}
              />
              <label
                htmlFor="dispatch-assignment-notes"
                className="grid gap-1 text-sm"
              >
                Notes
                <Textarea
                  id="dispatch-assignment-notes"
                  maxLength={2048}
                  rows={3}
                  value={notes}
                  onChange={(event) => setNotes(event.target.value)}
                />
              </label>
              <details
                open={advanced}
                onToggle={(event) => setAdvanced(event.currentTarget.open)}
              >
                <summary className="cursor-pointer text-sm">Advanced</summary>
                <div className="mt-3 space-y-3">
                  {reasons.length ? (
                    <ul className="list-disc pl-4 text-sm text-warning">
                      {reasons.map((reason) => (
                        <li key={reason}>{reason}</li>
                      ))}
                    </ul>
                  ) : null}
                  <label
                    htmlFor="dispatch-routing-override"
                    className="grid gap-1 text-sm"
                  >
                    Routing override
                    <Textarea
                      id="dispatch-routing-override"
                      maxLength={2000}
                      value={override}
                      onChange={(event) => setOverride(event.target.value)}
                    />
                  </label>
                  <label
                    htmlFor="dispatch-assignment-deadline"
                    className="grid gap-1 text-sm"
                  >
                    Deadline
                    <input
                      id="dispatch-assignment-deadline"
                      type="datetime-local"
                      className="h-9 rounded border border-input bg-background px-2"
                      value={deadline}
                      onChange={(event) => setDeadline(event.target.value)}
                    />
                  </label>
                </div>
              </details>
              {reasons.length > 0 && !override.trim() ? (
                <p className="text-sm text-warning">
                  Review the routing conditions in Advanced and explain an
                  override before assigning.
                </p>
              ) : null}
              {!busy && base.tipEvent !== issue.tipEvent ? (
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
                  (reasons.length > 0 && !override.trim()) ||
                  base.tipEvent !== issue.tipEvent
                }
              >
                {busy ? "Confirming assignment…" : "Assign"}
              </Button>
            </fieldset>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
