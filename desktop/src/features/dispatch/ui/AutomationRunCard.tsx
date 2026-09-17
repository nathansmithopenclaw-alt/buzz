import * as React from "react";
import { CirclePause, Play, RotateCcw, Settings2 } from "lucide-react";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/shared/ui/dialog";
import { automationState, automationTitle } from "../automationPresentation";
import { automationPreview } from "../automationPreview";
import { publishPlanningRecord } from "../recordMutations";
import {
  activeRecords,
  type AutomationControls,
  type RecordOf,
} from "../recordTypes";
import type { DispatchContext } from "../types";
import { AutomationSummary } from "./AutomationSummary";

export function AutomationRunCard({
  value,
  context,
  workspace,
  refresh,
  onEdit,
  onOpenIssue,
}: {
  value: RecordOf<"automation">;
  context: DispatchContext;
  workspace: string;
  refresh: () => Promise<DispatchContext>;
  onEdit: () => void;
  onOpenIssue: (id: string) => void;
}) {
  const controls = value.record.payload.controls;
  const [now, setNow] = React.useState(Date.now() / 1000);
  React.useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now() / 1000), 10000);
    return () => window.clearInterval(timer);
  }, []);
  const state = automationState(context, value, now);
  const [busy, setBusy] = React.useState(false),
    [confirm, setConfirm] = React.useState(false),
    [error, setError] = React.useState<string | null>(null);
  const editable =
    context.viewerRole === "maintainer" && !context.sectionErrors.records;
  const preview = automationPreview(context, controls);
  const changeMode = async (mode: AutomationControls["mode"]) => {
    setBusy(true);
    setError(null);
    try {
      // Re-read before enabling: the displayed connection/allowance can have aged.
      const latest = await refresh();
      const current = activeRecords(latest.records, "automation").find(
        (item) =>
          item.record.payload.controls.service_id === controls.service_id &&
          item.record.payload.controls.service_pubkey ===
            controls.service_pubkey,
      );
      if (current?.event_id !== value.event_id)
        throw new Error(
          "These settings changed. Review the updated setup and try again.",
        );
      if (
        mode === "active" &&
        !automationState(latest, value, Date.now() / 1000).canStart
      )
        throw new Error(
          "Buzz is not ready to turn this on yet. Check the connection and run limit, then try again.",
        );
      await publishPlanningRecord({
        workspace,
        previous: value,
        payload: {
          type: "automation",
          controls: {
            ...controls,
            mode,
            expires_at:
              mode === "active"
                ? Math.floor(Date.now() / 1000) + 3600
                : controls.expires_at,
          },
        },
        refresh,
      });
      setConfirm(false);
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section
      className="overflow-hidden rounded-xl border border-border bg-card"
      data-testid="automation-run-card"
    >
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border/60 p-5">
        <div className="min-w-0 space-y-2">
          <h3 className="font-semibold">{automationTitle(controls)}</h3>
          <span
            className={`inline-flex rounded-full px-2.5 py-1 text-xs font-medium ${state.label === "On" ? "bg-primary/10 text-primary" : "bg-muted text-foreground"}`}
            role="status"
          >
            {state.label}
          </span>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={busy || !editable}
            onClick={onEdit}
          >
            {state.remaining === 0 || state.expired ? (
              <RotateCcw className="size-4" aria-hidden="true" />
            ) : (
              <Settings2 className="size-4" aria-hidden="true" />
            )}
            {state.remaining === 0 || state.expired
              ? "Review another run"
              : "Edit setup"}
          </Button>
          {controls.mode === "active" ||
          (controls.mode === "preview" && !state.acknowledged) ? (
            <Button
              size="sm"
              variant="outline"
              disabled={busy || !editable}
              onClick={() => void changeMode("paused")}
            >
              <CirclePause className="size-4" aria-hidden="true" />
              {busy ? "Saving…" : "Pause"}
            </Button>
          ) : null}
          {controls.mode === "paused" && state.remaining !== 0 ? (
            <Button
              size="sm"
              variant="outline"
              disabled={busy || !editable}
              onClick={() => void changeMode("preview")}
            >
              {busy ? "Checking…" : "Check before turning on"}
            </Button>
          ) : null}
          {controls.mode === "preview" && state.remaining !== 0 ? (
            <Button
              size="sm"
              disabled={busy || !editable || !state.canStart}
              onClick={() => setConfirm(true)}
            >
              <Play className="size-4" aria-hidden="true" />
              Turn on…
            </Button>
          ) : null}
        </div>
      </div>
      <div className="space-y-4 p-5">
        <p className="text-sm text-muted-foreground">{state.explanation}</p>
        <AutomationSummary controls={controls} />
        <div className="rounded-lg bg-muted/40 p-3 text-sm">
          <p className="font-medium">
            {state.used === null
              ? `Up to ${controls.max_actions_total} actions per run`
              : `${state.used} of ${controls.max_actions_total} actions used`}
          </p>
          <p className="mt-1 text-muted-foreground">
            Each saved issue, assignment, or reminder counts as one action.
            Pausing does not reset this limit.
          </p>
          {state.pending ? (
            <p className="mt-1">
              Still confirming {state.pending}{" "}
              {state.pending === 1 ? "action" : "actions"}. These already count
              toward the limit.
            </p>
          ) : null}
          {controls.mode === "active" && controls.expires_at !== null ? (
            <p className="mt-1">
              Stops {new Date(controls.expires_at * 1000).toLocaleString()} or
              when the limit is reached.
            </p>
          ) : (
            <p className="mt-1">
              When turned on, this run stops after one hour or when the limit is
              reached.
            </p>
          )}
          {state.remaining === 0 ? (
            <p className="mt-2 font-medium">
              To do more, choose “Review another run” and check the work and
              limit again.
            </p>
          ) : null}
        </div>
        {!state.permissionReady ? (
          <p className="text-sm text-muted-foreground">
            Buzz is still checking access to this workspace. You can edit the
            setup or pause it while that completes.
          </p>
        ) : null}
        {controls.mode === "preview" &&
        state.acknowledged &&
        (state.used === null || state.pending === null) ? (
          <p className="text-sm text-muted-foreground">
            Waiting for Buzz to confirm how much of this run is left before it
            can start.
          </p>
        ) : null}
        {preview.length ? (
          <details className="rounded-lg border border-border p-3">
            <summary className="cursor-pointer text-sm font-medium">
              Selected work · {preview.length}{" "}
              {preview.length === 1 ? "issue" : "issues"}
            </summary>
            <p className="mt-2 text-sm text-muted-foreground">
              This is a preview of current work. Buzz checks again before it
              acts.
            </p>
            <ul className="mt-3 max-h-64 space-y-3 overflow-y-auto">
              {preview.map((item) => (
                <li key={item.id} className="text-sm">
                  <button
                    type="button"
                    className="text-left font-medium hover:underline"
                    onClick={() => onOpenIssue(item.id)}
                  >
                    {item.title}
                  </button>
                  <p className="mt-1 text-muted-foreground">{item.reason}</p>
                </li>
              ))}
            </ul>
          </details>
        ) : null}
        {controls.capture_channels.length ? (
          <p className="text-sm text-muted-foreground">
            Only 📌 reactions added after this setup was saved are included.
            Buzz skips messages that cannot be shared with everyone in this
            Dispatch workspace.
          </p>
        ) : null}
        <details className="text-xs text-muted-foreground">
          <summary className="cursor-pointer">
            Connection details for administrators
          </summary>
          <dl className="mt-2 space-y-1 break-all">
            <dt>Service</dt>
            <dd>{controls.service_id}</dd>
            <dt>Public key</dt>
            <dd>{controls.service_pubkey}</dd>
            <dt>Last check</dt>
            <dd>
              {state.beat
                ? new Date(state.beat.observedAt * 1000).toLocaleString()
                : "Not connected"}
            </dd>
            <dt>Diagnostic</dt>
            <dd>{state.beat?.health.detail ?? "No diagnostic received"}</dd>
          </dl>
        </details>
        {error && !confirm ? (
          <div role="alert" className="text-sm text-destructive">
            Buzz could not confirm the change. Check the connection and try
            again.
            <details className="mt-2 text-xs">
              <summary>Show details</summary>
              {error}
            </details>
          </div>
        ) : null}
      </div>
      <Dialog
        open={confirm}
        onOpenChange={(open) => {
          if (!busy) setConfirm(open);
        }}
      >
        <DialogContent className="max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Turn on this automation?</DialogTitle>
            <DialogDescription>
              Buzz can start the work below as soon as you turn it on.
            </DialogDescription>
          </DialogHeader>
          <AutomationSummary controls={controls} />
          <p className="rounded-lg bg-muted/40 p-3 text-sm">
            Stop after up to{" "}
            <strong>
              {state.remaining ?? controls.max_actions_total} more actions
            </strong>{" "}
            or <strong>one hour</strong>, whichever comes first. You can pause
            at any time.
          </p>
          {controls.routing.length ? (
            <p className="text-sm text-muted-foreground">
              This can wake the agents you chose. You will still review their
              results.
            </p>
          ) : null}
          {!state.canStart ? (
            <p role="status" className="text-sm">
              The setup needs another check before it can start. Close this
              window and check its status.
            </p>
          ) : null}
          {error ? (
            <div role="alert" className="text-sm text-destructive">
              Buzz could not confirm that automation is on. Check its status
              before trying again.
              <details className="mt-2 text-xs">
                <summary>Show details</summary>
                {error}
              </details>
            </div>
          ) : null}
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => setConfirm(false)}
            >
              Not yet
            </Button>
            <Button
              disabled={busy || !state.canStart}
              onClick={() => void changeMode("active")}
            >
              {busy ? "Turning on…" : "Turn on for up to 1 hour"}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </section>
  );
}
