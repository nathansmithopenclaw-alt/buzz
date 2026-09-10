import { useDispatchJournalVersion } from "./useDispatchJournalVersion";
import { KeepAsDraftButton } from "./DispatchDraftsPanel";
import { keepTaskReviewAsDraft } from "../taskReviews";
import * as React from "react";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import {
  pendingTaskReview,
  retryTaskReview,
  reviewDispatchTask,
  type PendingTaskReview,
} from "../taskReviews";
import type { DispatchContext, DispatchLinkedTask } from "../types";
export function TaskReviewDialog({
  task,
  workspace,
  viewer,
  refresh,
}: {
  task: DispatchLinkedTask;
  workspace: string;
  viewer?: string;
  refresh: () => Promise<DispatchContext>;
}) {
  const journalVersion = useDispatchJournalVersion();
  const [opened, setOpened] = React.useState<DispatchLinkedTask | null>(null),
    [evidence, setEvidence] = React.useState(""),
    [notes, setNotes] = React.useState(""),
    [busy, setBusy] = React.useState(false),
    [error, setError] = React.useState<string | null>(null),
    [pending, setPending] = React.useState<PendingTaskReview | null>(null);
  const formId = React.useId();
  // biome-ignore lint/correctness/useExhaustiveDependencies: Journal writes invalidate the durable data read by this effect.
  React.useEffect(() => {
    if (!task.eventId) return;
    let current = true;
    void pendingTaskReview(workspace, task.taskId)
      .then((value) => {
        if (current) setPending(value);
      })
      .catch((error) => {
        if (current) setError(String(error));
      });
    return () => {
      current = false;
    };
  }, [workspace, task.taskId, task.eventId, journalVersion]);
  const allowed =
    task.status === "done" &&
    viewer !== task.assignee &&
    (viewer === task.assigner || viewer === task.reviewer);
  const save = async (accepted: boolean) => {
    if (!opened) return;
    setBusy(true);
    setError(null);
    try {
      await reviewDispatchTask({
        workspace,
        task: opened,
        accepted,
        evidence,
        notes,
        refresh,
      });
      setOpened(null);
      setEvidence("");
      setNotes("");
    } catch (error) {
      setError(String(error));
      void pendingTaskReview(workspace, task.taskId)
        .then(setPending)
        .catch((recoveryError) =>
          setError(`${String(error)} ${String(recoveryError)}`),
        );
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="mt-3">
      {allowed ? (
        <Button size="sm" variant="outline" onClick={() => setOpened(task)}>
          Review reported result
        </Button>
      ) : null}
      {pending ? (
        <div className="mt-2 rounded border border-warning/50 p-3 text-sm">
          <p>A review is saved on this device.</p>
          <p className="mt-1 whitespace-pre-wrap">{pending.notes}</p>
          <p className="break-all text-xs">Evidence: {pending.evidence}</p>
          <Button
            className="mt-2"
            size="sm"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              setError(null);
              void retryTaskReview(pending, refresh)
                .then(() => {
                  setPending(null);
                  setOpened(null);
                })
                .catch((error) => setError(String(error)))
                .finally(() => setBusy(false));
            }}
          >
            Check and retry review
          </Button>
          <KeepAsDraftButton
            disabled={busy}
            onKeep={async () => {
              await keepTaskReviewAsDraft(pending);
              setPending(null);
            }}
          />
        </div>
      ) : null}
      {!opened && error ? (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}
      <Dialog
        open={Boolean(opened)}
        onOpenChange={(open) => {
          if (!open && !busy) setOpened(null);
        }}
      >
        <DialogContent className="max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Review the reported result</DialogTitle>
            <DialogDescription>
              Check the exact deliverables against the agreed review method.
              This records your review; issue status remains a separate
              decision.
            </DialogDescription>
          </DialogHeader>
          <p className="text-sm">{opened?.verification?.method}</p>
          <label htmlFor={`${formId}-evidence`} className="grid gap-2 text-sm">
            Review evidence
            <Input
              id={`${formId}-evidence`}
              value={evidence}
              onChange={(event) => setEvidence(event.target.value)}
              placeholder="Link or reference to your review receipt"
              maxLength={2048}
              disabled={busy || Boolean(pending)}
            />
          </label>
          <label htmlFor={`${formId}-notes`} className="grid gap-2 text-sm">
            What did you check?
            <Textarea
              id={`${formId}-notes`}
              value={notes}
              onChange={(event) => setNotes(event.target.value)}
              placeholder="Observed results, or the criteria that still need work"
              maxLength={8192}
              disabled={busy || Boolean(pending)}
            />
          </label>
          {error ? (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          ) : null}
          {pending ? (
            <p className="text-sm">
              Your review is saved. Close this dialog and use “Check and retry
              review” to confirm it.
            </p>
          ) : null}
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              variant="outline"
              disabled={
                busy || Boolean(pending) || !evidence.trim() || !notes.trim()
              }
              onClick={() => void save(false)}
            >
              Reject result
            </Button>
            <Button
              disabled={
                busy || Boolean(pending) || !evidence.trim() || !notes.trim()
              }
              onClick={() => void save(true)}
            >
              {busy ? "Confirming…" : "Accept reviewed result"}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
