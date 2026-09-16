import * as React from "react";
import { Button } from "@/shared/ui/button";
import {
  dispatchDrafts,
  dispatchDraftText,
  removeDispatchDraft,
  watchDispatchDrafts,
  type DispatchDraft,
} from "../dispatchDrafts";
import { buildDispatchLink } from "../dispatchLink";
import { DispatchLinkAnchor } from "./DispatchLinkAnchor";

export function KeepAsDraftButton({
  onKeep,
  disabled,
}: {
  onKeep: () => Promise<unknown>;
  disabled?: boolean;
}) {
  const [busy, setBusy] = React.useState(false),
    [error, setError] = React.useState<string | null>(null);
  return (
    <span className="inline-flex flex-col items-start gap-1">
      <Button
        variant="ghost"
        size="sm"
        disabled={disabled || busy}
        onClick={() => {
          setBusy(true);
          setError(null);
          void onKeep()
            .catch((error) => setError(String(error)))
            .finally(() => setBusy(false));
        }}
      >
        Keep as draft
      </Button>
      {error ? (
        <span role="alert" className="text-xs text-destructive">
          {error}
        </span>
      ) : null}
    </span>
  );
}
export function DispatchDraftsPanel({
  workspace,
  snapshotAt,
}: {
  workspace: string;
  snapshotAt: string;
}) {
  const [drafts, setDrafts] = React.useState<DispatchDraft[]>([]),
    [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (!snapshotAt) return;
    let active = true;
    const update = () => {
      void dispatchDrafts(workspace)
        .then((value) => {
          if (active) setDrafts(value);
        })
        .catch((error) => {
          if (active) setError(String(error));
        });
    };
    update();
    const dispose = watchDispatchDrafts(update);
    return () => {
      active = false;
      dispose();
    };
  }, [workspace, snapshotAt]);
  if (!drafts.length && !error) return null;
  return (
    <details className="m-3 rounded border border-border p-3 text-sm">
      <summary>Saved drafts on this device ({drafts.length})</summary>
      <p className="my-2 text-xs text-muted-foreground">
        An earlier send may still arrive. Review current history before manually
        applying a draft. Keeping a draft stops its retry here; it does not undo
        a published change.
      </p>
      {drafts.map((draft) => (
        <div
          key={draft.event.id}
          className="mt-3 rounded border border-border p-3"
        >
          <p className="font-medium">{draft.category}</p>
          <p className="text-xs text-muted-foreground">
            Saved {new Date(draft.savedAt).toLocaleString()}
          </p>
          <pre className="my-2 max-h-44 overflow-auto whitespace-pre-wrap break-words font-sans">
            {dispatchDraftText(draft.event)}
          </pre>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                void navigator.clipboard
                  .writeText(dispatchDraftText(draft.event))
                  .catch((error) => setError(String(error)));
              }}
            >
              Copy draft
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                const url = URL.createObjectURL(
                  new Blob([JSON.stringify(draft, null, 2)], {
                    type: "application/json",
                  }),
                );
                const link = document.createElement("a");
                link.href = url;
                link.download = `dispatch-recovery-${draft.event.id.slice(0, 12)}.json`;
                link.click();
                setTimeout(() => URL.revokeObjectURL(url), 1000);
              }}
            >
              Export recovery copy
            </Button>
            {draft.issueId ? (
              <DispatchLinkAnchor
                href={buildDispatchLink({
                  workspace,
                  issue: draft.issueId,
                  relay: draft.relay,
                })}
              >
                Review current issue
              </DispatchLinkAnchor>
            ) : null}
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                void removeDispatchDraft(draft).catch((error) =>
                  setError(String(error)),
                );
              }}
            >
              Remove from device
            </Button>
          </div>
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
