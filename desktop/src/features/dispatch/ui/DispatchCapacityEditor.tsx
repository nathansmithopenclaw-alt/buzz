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
}: {
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
    resolveUserLabel({ pubkey: key, profiles: profiles.data?.profiles });
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
  return (
    <section className="rounded-xl border border-border p-4">
      <div className="flex items-center justify-between gap-3">
        <h2 className="font-semibold">Executor capacity</h2>
        <Button
          size="sm"
          variant="outline"
          disabled={!editable}
          onClick={() => edit()}
        >
          Add executor
        </Button>
      </div>
      <p className="mt-2 text-sm text-muted-foreground">
        Set explicit capabilities and simultaneous work limits. Pausing an
        executor holds new routing proposals.
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
                  : `${value.record.payload.max_active} active attempt(s) maximum`}
              </span>
              <Button
                size="sm"
                variant="ghost"
                disabled={!editable}
                onClick={() => edit(value)}
              >
                Edit capacity
              </Button>
            </div>
          ))
        ) : (
          <p className="text-sm text-muted-foreground">
            No executors enabled for automatic routing.
          </p>
        )}
      </div>
      <Dialog
        open={open}
        onOpenChange={(value) => {
          if (!busy) setOpen(value);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Executor capacity</DialogTitle>
            <DialogDescription>
              These settings guide proposals and bounded automation. Saving does
              not assign work.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={(event) => void save(event)}>
            <fieldset className="space-y-4" disabled={busy}>
              <Button
                type="button"
                variant="outline"
                disabled={Boolean(base)}
                onClick={() => setPicker(true)}
              >
                {executor ? label(executor) : "Choose executor"}
              </Button>
              <AssigneePickerDialog
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
                Maximum active attempts
                <Input
                  id="capacity-maximum"
                  aria-label="Maximum active attempts"
                  type="number"
                  min={1}
                  max={100}
                  value={limit}
                  onChange={(event) => setLimit(Number(event.target.value))}
                />
              </label>
              <label htmlFor="capacity-labels" className="grid gap-1 text-sm">
                Capability labels
                <Input
                  id="capacity-labels"
                  aria-label="Capability labels"
                  value={labels}
                  onChange={(event) => setLabels(event.target.value)}
                  placeholder="area:desktop, testing"
                />
              </label>
              <label className="flex gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={paused}
                  onChange={(event) => setPaused(event.target.checked)}
                />
                Pause new assignments
              </label>
              {error ? (
                <p role="alert" className="text-sm text-destructive">
                  {error}
                </p>
              ) : null}
              <Button type="submit" disabled={!executor || busy}>
                {busy ? "Saving…" : "Save capacity"}
              </Button>
            </fieldset>
          </form>
        </DialogContent>
      </Dialog>
    </section>
  );
}
