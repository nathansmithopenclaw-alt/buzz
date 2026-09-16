import { useDispatchJournalVersion } from "./useDispatchJournalVersion";
import { dispatchDraftText } from "../dispatchDrafts";
import { KeepAsDraftButton } from "./DispatchDraftsPanel";
import { keepIssueWriteAsDraft } from "../issueWriteRecovery";
import * as React from "react";
import { Button } from "@/shared/ui/button";
import {
  pendingIssueWrites,
  retryIssueWrite,
  type PendingIssueWrite,
} from "../issueWriteRecovery";
import type { DispatchContext } from "../types";
export function IssueWriteRecoveryPanel({
  workspace,
  snapshotAt,
  refresh,
}: {
  workspace: string;
  snapshotAt: string;
  refresh: () => Promise<DispatchContext>;
}) {
  const journalVersion = useDispatchJournalVersion();
  const [pending, setPending] = React.useState<PendingIssueWrite[]>([]),
    [busy, setBusy] = React.useState(false),
    [error, setError] = React.useState<string | null>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: Journal writes invalidate the durable data read by this effect.
  React.useEffect(() => {
    if (new Date(snapshotAt).valueOf() <= 0) return;
    let active = true;
    void pendingIssueWrites(workspace)
      .then((value) => {
        if (active) setPending(value);
      })
      .catch((error) => {
        if (active) setError(String(error));
      });
    return () => {
      active = false;
    };
  }, [workspace, snapshotAt, journalVersion]);
  if (!pending.length && !error) return null;
  return (
    <details className="m-3 rounded border border-warning/40 p-3 text-sm">
      <summary>{pending.length} issue change(s) awaiting confirmation</summary>
      <p className="mt-2 text-xs text-muted-foreground">
        These exact signed changes are saved on this device. A retry first
        checks whether the change already arrived.
      </p>
      {pending.map((value) => (
        <div
          key={value.event.id}
          className="mt-3 rounded border border-border p-2"
        >
          <p>
            {value.event.kind === 40020
              ? "New issue"
              : value.event.kind === 40021
                ? "Issue edit"
                : "Comment"}
          </p>
          <details className="my-2 text-xs">
            <summary>Review saved content</summary>
            <pre className="max-h-40 overflow-auto whitespace-pre-wrap">
              {dispatchDraftText(value.event)}
            </pre>
          </details>
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              setError(null);
              void retryIssueWrite(value, refresh)
                .then(() =>
                  setPending((current) =>
                    current.filter(
                      (entry) => entry.event.id !== value.event.id,
                    ),
                  ),
                )
                .catch((error) => setError(String(error)))
                .finally(() => setBusy(false));
            }}
          >
            Check and retry saved change
          </Button>
          <KeepAsDraftButton
            disabled={busy}
            onKeep={async () => {
              await keepIssueWriteAsDraft(value);
              setPending((current) =>
                current.filter((entry) => entry.event.id !== value.event.id),
              );
            }}
          />
        </div>
      ))}
      {error ? (
        <p role="alert" className="mt-2 text-destructive">
          {error}
        </p>
      ) : null}
    </details>
  );
}
