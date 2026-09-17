import * as React from "react";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/shared/ui/dialog";
import { activeRecords, type RecordOf } from "../recordTypes";
import { publishPlanningRecord } from "../recordMutations";
import type { DispatchContext } from "../types";
import { AssigneePickerDialog } from "./AssigneePickerDialog";
export function DispatchCapacityEditor({
  context,
  workspace,
  refresh,
  inline = false,
}: {
  inline?: boolean;
  context: DispatchContext;
  workspace: string;
  refresh: () => Promise<DispatchContext>;
}) {
  const records = activeRecords(context.records, "capacity");
  const [open, setOpen] = React.useState(false),
    [picker, setPicker] = React.useState(false),
    [busy, setBusy] = React.useState(false);
  const [base, setBase] = React.useState<RecordOf<"capacity"> | undefined>(),
    [executor, setExecutor] = React.useState<string | null>(null),
    [limit, setLimit] = React.useState(1),
    [labels, setLabels] = React.useState(""),
    [paused, setPaused] = React.useState(false),
    [error, setError] = React.useState<string | null>(null);
  const profiles = useUsersBatchQuery([
    ...new Set([
      ...records.map((value) => value.record.payload.executor),
      ...(executor ? [executor] : []),
    ]),
  ]);
  const label = (key: string) =>
    resolveUserLabel({
      pubkey: key,
      profiles: profiles.data?.profiles,
      fallbackName: "Agent name unavailable",
    });
  const editable =
    context.viewerRole === "maintainer" && !context.sectionErrors.records;
  const edit = (value?: RecordOf<"capacity">) => {
    setBase(value);
    setExecutor(value?.record.payload.executor ?? null);
    setLimit(value?.record.payload.max_active ?? 1);
    setLabels(value?.record.payload.capabilities.join(", ") ?? "");
    setPaused(value?.record.payload.paused ?? false);
    setError(null);
    setOpen(true);
  };
  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!executor) return;
    setBusy(true);
    setError(null);
    try {
      await publishPlanningRecord({
        workspace,
        previous: base,
        payload: {
          type: "capacity",
          executor,
          max_active: limit,
          capabilities: [
            ...new Set(
              labels
                .split(",")
                .map((label) => label.trim().toLowerCase())
                .filter(Boolean),
            ),
          ],
          paused,
        },
        refresh,
      });
      setOpen(false);
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  const description =
    "Tell Buzz what this agent can handle. Automations that are already on will use these settings. Pausing stops new assignments, but work already started can continue.";
  const form = (
    <form onSubmit={(event) => void save(event)}>
      <fieldset className="space-y-4" disabled={busy}>
        <Button
          type="button"
          variant="outline"
          disabled={Boolean(base)}
          onClick={() => setPicker(true)}
        >
          {executor ? label(executor) : "Choose an agent"}
        </Button>
        <AssigneePickerDialog
          agentsOnly
          open={picker}
          onOpenChange={setPicker}
          assignee={executor}
          disabled={busy}
          onAssign={(value) => {
            setExecutor(value);
            setPicker(false);
          }}
        />
        <label htmlFor="capacity-maximum" className="grid gap-1 text-sm">
          Issues at the same time
          <Input
            id="capacity-maximum"
            aria-label="Issues at the same time"
            required
            type="number"
            min={1}
            max={100}
            value={limit}
            onChange={(event) => setLimit(Number(event.target.value))}
          />
        </label>
        <label htmlFor="capacity-labels" className="grid gap-1 text-sm">
          Issue labels this agent can handle
          <Input
            id="capacity-labels"
            aria-label="Issue labels this agent can handle"
            value={labels}
            onChange={(event) => setLabels(event.target.value)}
            placeholder="For example: research, writing"
          />
        </label>
        <p className="text-xs text-muted-foreground">
          Use the labels on your issues, separated by commas. An assignment rule
          must match one of these labels.
        </p>
        <label className="flex gap-2 text-sm">
          <input
            type="checkbox"
            checked={paused}
            onChange={(event) => setPaused(event.target.checked)}
          />
          Pause new assignments
        </label>
        {error ? (
          <div role="alert" className="text-sm text-destructive">
            Buzz could not save these agent settings. Your choices are still
            here; check the connection and try again.
            <details className="mt-2 text-xs">
              <summary>Show details</summary>
              {error}
            </details>
          </div>
        ) : null}
        <div className="flex flex-wrap gap-2">
          <Button type="submit" disabled={!executor || busy}>
            {busy ? "Saving…" : "Save agent settings"}
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={busy}
            onClick={() => setOpen(false)}
          >
            Cancel agent settings
          </Button>
        </div>
      </fieldset>
    </form>
  );
  return (
    <section className="rounded-xl border border-border p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="font-semibold">Agents that can take work</h2>
        <Button
          size="sm"
          variant="outline"
          disabled={!editable}
          onClick={() => edit()}
        >
          Add an agent
        </Button>
      </div>
      <p className="mt-2 text-sm text-muted-foreground">
        Choose what each agent can work on and how many issues it can handle at
        once. Pause an agent here to stop new automatic assignments.
      </p>
      <div className="mt-3 space-y-2">
        {records.length ? (
          records.map((value) => (
            <div
              key={value.event_id}
              className="flex items-center justify-between gap-3 text-sm"
            >
              <span>
                {label(value.record.payload.executor)} ·{" "}
                {value.record.payload.paused
                  ? "Paused"
                  : `Up to ${value.record.payload.max_active} ${value.record.payload.max_active === 1 ? "issue" : "issues"} at a time`}
              </span>
              <Button
                size="sm"
                variant="ghost"
                disabled={!editable}
                onClick={() => edit(value)}
              >
                Edit agent settings
              </Button>
            </div>
          ))
        ) : (
          <p className="text-sm text-muted-foreground">
            No agents are set up to take work automatically yet.
          </p>
        )}
      </div>
      {inline && open ? (
        <section
          aria-label="Agent work settings"
          className="mt-4 space-y-4 border-t border-border pt-4"
        >
          <h3 className="font-semibold">Agent work settings</h3>
          <p className="text-sm text-muted-foreground">{description}</p>
          {form}
        </section>
      ) : null}
      {!inline ? (
        <Dialog
          open={open}
          onOpenChange={(value) => {
            if (!busy) setOpen(value);
          }}
        >
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Agent work settings</DialogTitle>
              <DialogDescription>{description}</DialogDescription>
            </DialogHeader>
            {form}
          </DialogContent>
        </Dialog>
      ) : null}
    </section>
  );
}
