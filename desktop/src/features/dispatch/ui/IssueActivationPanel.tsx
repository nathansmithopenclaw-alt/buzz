import * as React from "react";
import { Button } from "@/shared/ui/button";
import { Textarea } from "@/shared/ui/textarea";
import { activeRecords } from "../recordTypes";
import { publishPlanningRecord } from "../recordMutations";
import type { DispatchContext, DispatchIssue } from "../types";

export function IssueActivationPanel({
  issue,
  context,
  refresh,
}: {
  issue: DispatchIssue;
  context: DispatchContext;
  refresh: () => Promise<DispatchContext>;
}) {
  const [editing, setEditing] = React.useState(false);
  const [reason, setReason] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const imported = context.activity.some(
    (entry) => entry.issueId === issue.issueId && entry.import,
  );
  const previous = activeRecords(context.records, "activation").find(
    (value) => value.record.payload.issue_id === issue.issueId,
  );
  const [base, setBase] = React.useState(previous);
  if (!imported) return null;
  const active = previous?.record.payload.active ?? false;
  return (
    <section
      className="mt-4 rounded-lg border border-border bg-muted/20 p-3"
      aria-label="Imported work activation"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 className="text-sm font-medium">
            {active ? "Imported work · activated" : "Imported history · parked"}
          </h2>
          <p className="mt-1 text-xs text-muted-foreground">
            {active
              ? "A maintainer has accepted this issue into current work."
              : "This issue remains searchable. Accept it into current work before it can enter the ready queue or automation."}
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={
            context.viewerRole !== "maintainer" ||
            busy ||
            Boolean(context.sectionErrors.records)
          }
          onClick={() => {
            setBase(previous);
            setEditing(true);
          }}
        >
          {active ? "Park this work" : "Accept into current work"}
        </Button>
      </div>
      {previous ? (
        <p className="mt-2 text-xs">{previous.record.payload.reason}</p>
      ) : null}
      {editing ? (
        <form
          className="mt-3 space-y-2"
          onSubmit={(event) => {
            event.preventDefault();
            setBusy(true);
            setError(null);
            void publishPlanningRecord({
              workspace: issue.streamId,
              previous: base,
              payload: {
                type: "activation",
                issue_id: issue.issueId,
                active: !(base?.record.payload.active ?? false),
                reason: reason.trim(),
              },
              refresh,
            })
              .then(() => {
                setEditing(false);
                setReason("");
              })
              .catch((error) => setError(String(error)))
              .finally(() => setBusy(false));
          }}
        >
          <label className="grid gap-1 text-sm" htmlFor="activation-reason">
            Reason
            <Textarea
              id="activation-reason"
              value={reason}
              required
              maxLength={2000}
              disabled={busy}
              onChange={(event) => setReason(event.target.value)}
            />
          </label>
          {base?.event_id !== previous?.event_id ? (
            <p role="status" className="text-sm text-warning">
              Activation changed. Review the current state and choose the action
              again; your reason is retained.
            </p>
          ) : null}
          <div className="flex gap-2">
            <Button
              type="submit"
              size="sm"
              disabled={
                busy || !reason.trim() || base?.event_id !== previous?.event_id
              }
            >
              {busy ? "Saving…" : "Confirm activation change"}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => setEditing(false)}
            >
              Cancel
            </Button>
          </div>
        </form>
      ) : null}
      {error ? (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </section>
  );
}
