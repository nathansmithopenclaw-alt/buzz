import * as React from "react";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Textarea } from "@/shared/ui/textarea";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { getRelayWsUrl } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import {
  canApplyDispatchMutation,
  publishDispatchIssueMutation,
} from "../mutations";
import { issueAlias } from "../issueEditing";
import { STATUS_ORDER, statusLabel } from "../viewModel";
import type {
  DispatchContext,
  DispatchIssue,
  DispatchStatus,
  DispatchPriority,
} from "../types";
import { AssigneePickerDialog } from "./AssigneePickerDialog";

export function DispatchBulkActions({
  selected,
  context,
  onClear,
  refresh,
}: {
  selected: DispatchIssue[];
  context: DispatchContext;
  onClear: () => void;
  refresh: () => Promise<DispatchContext>;
}) {
  const [open, setOpen] = React.useState(false),
    [busy, setBusy] = React.useState(false),
    [picker, setPicker] = React.useState(false);
  const [base, setBase] = React.useState<DispatchIssue[]>([]),
    [status, setStatus] = React.useState<DispatchStatus | "keep">("keep"),
    [priority, setPriority] = React.useState<
      DispatchPriority | "none" | "keep"
    >("keep");
  const [owner, setOwner] = React.useState<string | null | undefined>(
      undefined,
    ),
    [reason, setReason] = React.useState("");
  const [results, setResults] = React.useState<
    { title: string; message: string; ok: boolean }[]
  >([]);
  const profiles = useUsersBatchQuery(owner ? [owner] : []);
  if (!selected.length && !open) return null;
  const fields = {
    ...(status === "keep" ? {} : { status }),
    ...(priority === "keep"
      ? {}
      : { priority: priority === "none" ? null : priority }),
    ...(owner === undefined ? {} : { assignee: owner }),
  };
  const apply = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setResults([]);
    try {
      const [relay, identity] = await Promise.all([
        getRelayWsUrl(),
        getIdentity(),
      ]);
      for (const issue of base) {
        try {
          const [currentRelay, currentIdentity] = await Promise.all([
            getRelayWsUrl(),
            getIdentity(),
          ]);
          if (
            currentRelay !== relay ||
            currentIdentity.pubkey !== identity.pubkey
          )
            throw new Error("Community or identity changed; batch stopped.");
          const latest = await refresh(),
            current = latest.issues.find(
              (value) => value.issueId === issue.issueId,
            );
          if (!current || current.tipEvent !== issue.tipEvent)
            throw new Error(
              "Changed since preview; review this issue before applying.",
            );
          const mutation = {
            type: "edit" as const,
            expectedTip: issue.tipEvent,
            fields,
            reason: reason.trim() || "Bulk update from Dispatch",
          };
          if (!canApplyDispatchMutation(current, mutation, latest.viewerRole))
            throw new Error("Your role does not permit this change.");
          await publishDispatchIssueMutation({
            issue: current,
            mutation,
            viewerRole: latest.viewerRole,
            onOptimistic: () => {},
            onRollback: () => {},
            refresh,
          });
          setResults((values) => [
            ...values,
            { title: issue.title, message: "Confirmed", ok: true },
          ]);
        } catch (error) {
          setResults((values) => [
            ...values,
            { title: issue.title, message: String(error), ok: false },
          ]);
          break;
        }
      }
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex flex-wrap items-center gap-3 border-b border-border bg-muted/30 px-5 py-2 text-sm">
      <span>{selected.length} selected</span>
      <Button
        variant="outline"
        size="sm"
        onClick={() => {
          setBase([...selected]);
          setResults([]);
          setOpen(true);
        }}
      >
        Edit selected issues
      </Button>
      <Button variant="ghost" size="sm" onClick={onClear}>
        Clear selection
      </Button>
      <Dialog
        open={open}
        onOpenChange={(value) => {
          if (!busy) setOpen(value);
        }}
      >
        <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Edit {base.length} selected issues</DialogTitle>
            <DialogDescription>
              Review the changes below. Each issue is confirmed separately. The
              batch stops at the first error and keeps its completed results.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={(event) => void apply(event)}>
            <fieldset disabled={busy} className="space-y-3">
              <div className="grid gap-3 sm:grid-cols-2">
                <label className="grid gap-1 text-sm">
                  Status
                  <select
                    aria-label="Status"
                    className="h-9 rounded border border-input bg-background px-2"
                    value={status}
                    onChange={(event) =>
                      setStatus(event.target.value as DispatchStatus | "keep")
                    }
                  >
                    <option value="keep">Keep current status</option>
                    {STATUS_ORDER.map((status) => (
                      <option key={status} value={status}>
                        {statusLabel(status)}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="grid gap-1 text-sm">
                  Priority
                  <select
                    aria-label="Priority"
                    className="h-9 rounded border border-input bg-background px-2"
                    value={priority}
                    onChange={(event) =>
                      setPriority(event.target.value as typeof priority)
                    }
                  >
                    <option value="keep">Keep current priority</option>
                    <option value="none">No priority</option>
                    {["P0", "P1", "P2", "P3"].map((priority) => (
                      <option key={priority}>{priority}</option>
                    ))}
                  </select>
                </label>
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setPicker(true)}
                >
                  Owner:{" "}
                  {owner === undefined
                    ? "Keep current"
                    : owner
                      ? resolveUserLabel({
                          pubkey: owner,
                          profiles: profiles.data?.profiles,
                        })
                      : "Unassigned"}
                </Button>
                {owner !== undefined ? (
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={() => setOwner(undefined)}
                  >
                    Keep current owners
                  </Button>
                ) : null}
              </div>
              <AssigneePickerDialog
                open={picker}
                onOpenChange={setPicker}
                assignee={owner ?? null}
                disabled={busy}
                onAssign={(value) => {
                  setOwner(value);
                  setPicker(false);
                }}
              />
              <label htmlFor="bulk-reason" className="grid gap-1 text-sm">
                Reason
                <Textarea
                  id="bulk-reason"
                  value={reason}
                  onChange={(event) => setReason(event.target.value)}
                  maxLength={4000}
                />
              </label>
              <details>
                <summary className="text-sm">
                  Review selected issues · {base.length}
                </summary>
                <ul className="mt-2 max-h-48 overflow-y-auto text-xs">
                  {base.map((issue) => (
                    <li key={issue.issueId} className="py-1">
                      {issueAlias(issue, context.issues)} · {issue.title}
                    </li>
                  ))}
                </ul>
              </details>
              <Button
                type="submit"
                disabled={
                  busy || !Object.keys(fields).length || results.length > 0
                }
              >
                {busy ? "Applying…" : "Apply reviewed changes"}
              </Button>
            </fieldset>
          </form>
          {results.length ? (
            <div role="status" className="space-y-2">
              <p className="text-sm">
                {results.filter((result) => result.ok).length} of {base.length}{" "}
                confirmed
              </p>
              {results.map((result) => (
                <p
                  key={result.title}
                  className={result.ok ? "text-xs" : "text-xs text-destructive"}
                >
                  {result.title} · {result.message}
                </p>
              ))}
            </div>
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  );
}
