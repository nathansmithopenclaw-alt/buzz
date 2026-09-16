import * as React from "react";
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
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { activeRecords, type DeliveryStage } from "../recordTypes";
import { publishPlanningRecord } from "../recordMutations";
import type { DispatchContext, DispatchIssue } from "../types";

export const DELIVERY_LABELS: Record<DeliveryStage, string> = {
  reported_complete: "Reported complete",
  tested: "Tests",
  reviewed: "Review",
  merged: "Merge",
  installed: "Installation",
  accepted: "Acceptance",
};
export function DeliveryPanel({
  issue,
  context,
  refresh,
}: {
  issue: DispatchIssue;
  context: DispatchContext;
  refresh: () => Promise<DispatchContext>;
}) {
  const [open, setOpen] = React.useState(false);
  const [stage, setStage] = React.useState<DeliveryStage>("tested");
  const [artifact, setArtifact] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [passed, setPassed] = React.useState(true);
  const [task, setTask] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const entries = activeRecords(context.records, "delivery")
    .filter((value) => value.record.payload.issue_id === issue.issueId)
    .sort((a, b) => b.created_at - a.created_at);
  const profiles = useUsersBatchQuery([
    ...new Set(entries.map((value) => value.author)),
  ]);
  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await publishPlanningRecord({
        workspace: issue.streamId,
        payload: {
          type: "delivery",
          issue_id: issue.issueId,
          task_id: task || null,
          stage,
          artifact: artifact.trim(),
          description: description.trim(),
          passed,
          observed_at: Math.floor(Date.now() / 1000),
        },
        refresh,
      });
      setOpen(false);
      setArtifact("");
      setDescription("");
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section
      aria-label="Delivery evidence"
      className="mt-5 border-t border-border pt-5"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-sm font-semibold">Delivery evidence</h2>
        <Button
          size="sm"
          variant="ghost"
          disabled={
            context.viewerRole === "none" ||
            Boolean(context.sectionErrors.records)
          }
          onClick={() => setOpen(true)}
        >
          Record evidence
        </Button>
      </div>
      <p className="mt-1 text-xs text-muted-foreground">
        Tests, review, installation and acceptance each have their own evidence.
      </p>
      {!entries.length ? (
        <p className="mt-2 text-sm text-muted-foreground">
          No delivery observations recorded yet.
        </p>
      ) : (
        <div className="mt-3 space-y-3">
          {entries.map((value) => {
            const evidence = value.record.payload;
            return (
              <article
                key={value.record.record_id}
                className="rounded-lg border border-border p-3 text-sm"
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <strong>
                    {DELIVERY_LABELS[evidence.stage]} ·{" "}
                    {evidence.passed
                      ? "Passed / confirmed"
                      : "Failed / not accepted"}
                  </strong>
                  <span className="text-xs text-muted-foreground">
                    Recorded by{" "}
                    {resolveUserLabel({
                      pubkey: value.author,
                      profiles: profiles.data?.profiles,
                    })}
                  </span>
                </div>
                <p className="mt-2 whitespace-pre-wrap">
                  {evidence.description}
                </p>
                <p className="mt-2 break-all text-xs">
                  {/^https?:\/\//i.test(evidence.artifact) ? (
                    <a
                      href={evidence.artifact}
                      target="_blank"
                      rel="noreferrer"
                      className="text-primary underline"
                    >
                      {evidence.artifact}
                    </a>
                  ) : (
                    evidence.artifact
                  )}
                </p>
                <time className="mt-1 block text-xs text-muted-foreground">
                  {new Date(evidence.observed_at * 1000).toLocaleString()}
                </time>
              </article>
            );
          })}
        </div>
      )}
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Record delivery evidence</DialogTitle>
            <DialogDescription>
              Name the exact artifact or environment and describe what you
              observed.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={(event) => void save(event)}>
            <fieldset disabled={busy} className="space-y-3">
              <label className="grid gap-1 text-sm">
                Observation
                <select
                  className="h-9 rounded border border-input bg-background px-2"
                  value={stage}
                  onChange={(event) =>
                    setStage(event.target.value as DeliveryStage)
                  }
                >
                  {Object.entries(DELIVERY_LABELS).map(([value, label]) => (
                    <option
                      key={value}
                      value={value}
                      disabled={
                        ["reviewed", "accepted"].includes(value) &&
                        context.viewerRole !== "maintainer"
                      }
                    >
                      {label}
                    </option>
                  ))}
                </select>
              </label>
              <label className="grid gap-1 text-sm">
                Execution attempt
                <select
                  className="h-9 rounded border border-input bg-background px-2"
                  value={task}
                  onChange={(event) => setTask(event.target.value)}
                >
                  <option value="">Issue-level observation</option>
                  {context.linkedTasks
                    .filter((value) => value.issueId === issue.issueId)
                    .map((value) => (
                      <option key={value.taskId} value={value.taskId}>
                        {value.title} · {value.status.replaceAll("_", " ")}
                      </option>
                    ))}
                </select>
              </label>
              <label
                htmlFor="dispatch-evidence-artifact"
                className="grid gap-1 text-sm"
              >
                Artifact, commit, report or environment
                <Input
                  id="dispatch-evidence-artifact"
                  value={artifact}
                  maxLength={4096}
                  required
                  onChange={(event) => setArtifact(event.target.value)}
                />
              </label>
              <label
                htmlFor="dispatch-evidence-description"
                className="grid gap-1 text-sm"
              >
                What was checked and what happened
                <Textarea
                  id="dispatch-evidence-description"
                  value={description}
                  rows={5}
                  maxLength={16000}
                  required
                  onChange={(event) => setDescription(event.target.value)}
                />
              </label>
              <label className="flex gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={passed}
                  onChange={(event) => setPassed(event.target.checked)}
                />
                This specific check passed
              </label>
              {error ? (
                <p role="alert" className="text-sm text-destructive">
                  {error}
                </p>
              ) : null}
              <Button
                type="submit"
                disabled={busy || !artifact.trim() || !description.trim()}
              >
                {busy ? "Recording…" : "Save observation"}
              </Button>
            </fieldset>
          </form>
        </DialogContent>
      </Dialog>
    </section>
  );
}
