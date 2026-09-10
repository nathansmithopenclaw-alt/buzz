import { useDispatchJournalVersion } from "./useDispatchJournalVersion";
import { DispatchDraftsPanel } from "./DispatchDraftsPanel";
import { KeepAsDraftButton } from "./DispatchDraftsPanel";
import { keepPlanningChangeAsDraft } from "../recordMutations";
import { IssueWriteRecoveryPanel } from "./IssueWriteRecoveryPanel";
import * as React from "react";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";
import { ATTENTION_VIEWS, type AttentionView } from "../attention";
import { activeRecords } from "../recordTypes";
import {
  pendingPlanningChanges,
  publishPlanningRecord,
  retryPlanningChange,
  type PendingRecord,
} from "../recordMutations";
import type { DispatchContext } from "../types";
import { DISPATCH_SEARCH_KEYS } from "../dispatchSearch";

type SavedView = { id: string; name: string; filters: Record<string, string> };
export function DispatchViewToolbar({
  scope,
  workspace,
  viewer,
  context,
  values,
  onApply,
  refresh,
}: {
  scope: string;
  workspace: string;
  viewer: string;
  context: DispatchContext;
  values: Partial<Record<string, string | null>>;
  onApply: (patch: Record<string, string | null>) => void;
  refresh: () => Promise<DispatchContext>;
}) {
  const journalVersion = useDispatchJournalVersion();
  const key = `buzz-dispatch-views-v1:${scope}:${workspace}:${viewer}`;
  const [views, setViews] = React.useState<SavedView[]>([]);
  const [error, setError] = React.useState<string | null>(null);
  const [open, setOpen] = React.useState(false);
  const [name, setName] = React.useState("");
  const [shared, setShared] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [pending, setPending] = React.useState<PendingRecord[]>([]);
  React.useEffect(() => {
    try {
      const raw = getStorageItem(key);
      const value: unknown = raw ? JSON.parse(raw) : [];
      if (
        !Array.isArray(value) ||
        value.length > 50 ||
        value.some(
          (view) =>
            !view ||
            typeof view.id !== "string" ||
            typeof view.name !== "string" ||
            !view.filters ||
            typeof view.filters !== "object",
        )
      )
        throw new Error(
          "Saved views could not be read. Your shared views remain available.",
        );
      setViews(value);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  }, [key]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: Journal writes invalidate the durable data read by this effect.
  React.useEffect(() => {
    if (new Date(context.snapshotAt).valueOf() <= 0) return;
    let cancelled = false;
    void pendingPlanningChanges(workspace)
      .then((value) => {
        if (!cancelled) setPending(value);
      })
      .catch((error) => {
        if (!cancelled) setError(String(error));
      });
    return () => {
      cancelled = true;
    };
  }, [workspace, context.snapshotAt, journalVersion]);
  const sharedViews = activeRecords(context.records, "view");
  const apply = (filters: Record<string, string>) => {
    const patch: Record<string, string | null> = {};
    for (const key of DISPATCH_SEARCH_KEYS)
      if (key !== "workspace") patch[key] = filters[key] ?? null;
    onApply(patch);
  };
  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!name.trim()) return;
    setBusy(true);
    setError(null);
    const filters = Object.fromEntries(
      Object.entries(values).filter(
        ([key, value]) =>
          key !== "workspace" &&
          key !== "exclude" &&
          typeof value === "string" &&
          value,
      ),
    ) as Record<string, string>;
    try {
      if (shared)
        await publishPlanningRecord({
          workspace,
          payload: { type: "view", name: name.trim(), filters },
          refresh,
        });
      else {
        if (views.length >= 50)
          throw new Error("Remove an unused saved view before adding another.");
        const next = [
          ...views,
          { id: crypto.randomUUID(), name: name.trim(), filters },
        ];
        if (!setStorageItem(key, JSON.stringify(next)))
          throw new Error("This view could not be saved on this device.");
        setViews(next);
      }
      setOpen(false);
      setName("");
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };
  const remove = async (id: string) => {
    setBusy(true);
    setError(null);
    try {
      const shared = sharedViews.find((value) => value.record.record_id === id);
      if (shared)
        await publishPlanningRecord({
          workspace,
          previous: shared,
          payload: shared.record.payload,
          archived: true,
          refresh,
        });
      else {
        const next = views.filter((view) => view.id !== id);
        if (!setStorageItem(key, JSON.stringify(next)))
          throw new Error("Could not remove the saved view.");
        setViews(next);
      }
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="border-b border-border/40 px-5 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <label className="flex items-center gap-2 text-xs text-muted-foreground">
          Focus
          <select
            aria-label="Attention view"
            className="h-8 rounded-md border border-input bg-background px-2 text-sm text-foreground"
            value={values.attention ?? "all"}
            onChange={(event) =>
              onApply({
                attention:
                  event.target.value === "all" ? null : event.target.value,
                view: "list",
                exclude: null,
              })
            }
          >
            {Object.entries(ATTENTION_VIEWS).map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label className="sr-only" htmlFor="dispatch-saved-view">
          Saved view
        </label>
        <select
          id="dispatch-saved-view"
          className="h-8 max-w-56 rounded-md border border-input bg-background px-2 text-sm"
          value=""
          onChange={(event) => {
            const personal = views.find(
              (view) => view.id === event.target.value,
            );
            const shared = sharedViews.find(
              (view) => view.record.record_id === event.target.value,
            );
            if (personal) apply(personal.filters);
            if (shared) apply(shared.record.payload.filters);
          }}
        >
          <option value="" disabled>
            Open saved view…
          </option>
          <optgroup label="On this device">
            {views.map((view) => (
              <option key={view.id} value={view.id}>
                {view.name}
              </option>
            ))}
          </optgroup>
          <optgroup label="Shared with workspace">
            {sharedViews.map((view) => (
              <option key={view.record.record_id} value={view.record.record_id}>
                {view.record.payload.name}
              </option>
            ))}
          </optgroup>
        </select>
        <Button variant="ghost" size="sm" onClick={() => setOpen(true)}>
          Save or manage views
        </Button>
        {values.attention && values.attention !== "all" ? (
          <span className="text-xs text-muted-foreground">
            {ATTENTION_VIEWS[values.attention as AttentionView]}
          </span>
        ) : null}
      </div>
      {error ? (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}
      <DispatchDraftsPanel
        workspace={workspace}
        snapshotAt={context.snapshotAt}
      />
      <IssueWriteRecoveryPanel
        workspace={workspace}
        snapshotAt={context.snapshotAt}
        refresh={refresh}
      />
      {pending.length ? (
        <details className="mt-2 rounded border border-warning/40 p-2 text-sm">
          <summary>
            {pending.length} shared change(s) awaiting confirmation
          </summary>
          {pending.map((value) => (
            <div key={value.event.id} className="mt-2 flex items-center gap-2">
              <span>{value.record.payload.type.replaceAll("_", " ")}</span>
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => {
                  setBusy(true);
                  void retryPlanningChange(value, refresh)
                    .then(() =>
                      setPending((current) =>
                        current.filter(
                          (item) => item.event.id !== value.event.id,
                        ),
                      ),
                    )
                    .catch((error) => setError(String(error)))
                    .finally(() => setBusy(false));
                }}
              >
                Retry saved change
              </Button>
              <KeepAsDraftButton
                disabled={busy}
                onKeep={async () => {
                  await keepPlanningChangeAsDraft(value);
                  setPending((current) =>
                    current.filter(
                      (entry) => entry.event.id !== value.event.id,
                    ),
                  );
                }}
              />
            </div>
          ))}
        </details>
      ) : null}
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Saved views</DialogTitle>
            <DialogDescription>
              Keep the filters and layout that help you find your work.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={(event) => void save(event)} className="space-y-3">
            <label htmlFor="dispatch-view-name" className="grid gap-1 text-sm">
              View name
              <Input
                id="dispatch-view-name"
                value={name}
                maxLength={80}
                onChange={(event) => setName(event.target.value)}
                placeholder="This week’s customer work"
                required
              />
            </label>
            <label className="flex gap-2 text-sm">
              <input
                type="checkbox"
                checked={shared}
                disabled={
                  context.viewerRole === "none" ||
                  Boolean(context.sectionErrors.records)
                }
                onChange={(event) => setShared(event.target.checked)}
              />
              Share with this workspace
            </label>
            <Button type="submit" disabled={busy || !name.trim()}>
              {busy ? "Saving…" : "Save current view"}
            </Button>
          </form>
          <div className="space-y-2">
            {views.map((view) => (
              <div
                key={view.id}
                className="flex items-center justify-between gap-3 text-sm"
              >
                <span>
                  {view.name}{" "}
                  <span className="text-muted-foreground">· this device</span>
                </span>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => void remove(view.id)}
                >
                  Remove
                </Button>
              </div>
            ))}
            {sharedViews.map((view) => (
              <div
                key={view.record.record_id}
                className="flex items-center justify-between gap-3 text-sm"
              >
                <span>
                  {view.record.payload.name}{" "}
                  <span className="text-muted-foreground">· shared</span>
                </span>
                {view.creator === viewer ||
                context.viewerRole === "maintainer" ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => void remove(view.record.record_id)}
                  >
                    Archive
                  </Button>
                ) : null}
              </div>
            ))}
          </div>
          {error ? (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  );
}
