import * as React from "react";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  dispatchContextExport,
  relatedIssueSuggestions,
} from "../contextExport";
import { issueAlias } from "../issueEditing";
import type { DispatchContext, DispatchIssue } from "../types";
import type { DispatchIssueMutation } from "../mutations";

export function IssueContextTools({
  context,
  issue,
  disabled,
  onOpenIssue,
  onSave,
}: {
  context: DispatchContext;
  issue: DispatchIssue;
  disabled: boolean;
  onOpenIssue: (id: string) => void;
  onSave: (mutation: DispatchIssueMutation) => Promise<DispatchIssue>;
}) {
  const [open, setOpen] = React.useState(false);
  const [delta, setDelta] = React.useState(true);
  const [message, setMessage] = React.useState<string | null>(null);
  const lastAttempt = Math.max(
    0,
    ...context.linkedTasks
      .filter((task) => task.issueId === issue.issueId)
      .map((task) => task.assignedAt ?? 0),
  );
  const brief = dispatchContextExport(context, issue, delta ? lastAttempt : 0);
  const suggestions = relatedIssueSuggestions(issue, context.issues).filter(
    (value) =>
      !issue.relations.some(
        (relation) => relation.targetIssueId === value.issue.issueId,
      ),
  );
  return (
    <section className="mt-4 space-y-3">
      <Button size="sm" variant="outline" onClick={() => setOpen(true)}>
        Prepare agent handoff
      </Button>
      {suggestions.length ? (
        <details className="rounded-lg border border-border p-3 text-sm">
          <summary className="cursor-pointer">
            Related work to review · {suggestions.length}
          </summary>
          <p className="mt-2 text-xs text-muted-foreground">
            These titles share wording. Review the work before deciding whether
            it is a duplicate.
          </p>
          {suggestions.map((value) => (
            <div
              key={value.issue.issueId}
              className="mt-3 flex flex-wrap items-center gap-2"
            >
              <Button
                size="sm"
                variant="link"
                onClick={() => onOpenIssue(value.issue.issueId)}
              >
                {issueAlias(value.issue, context.issues)} · {value.issue.title}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={disabled}
                onClick={() => {
                  void onSave({
                    type: "edit",
                    expectedTip: issue.tipEvent,
                    fields: {
                      relations: [
                        ...issue.relations,
                        {
                          kind: "duplicate_of",
                          targetIssueId: value.issue.issueId,
                          targetRootEvent: value.issue.rootEvent,
                        },
                      ],
                    },
                    reason:
                      "Reviewed related work and linked this issue as a duplicate",
                  })
                    .then(() =>
                      setMessage(
                        "Duplicate relationship saved. The issue status is unchanged.",
                      ),
                    )
                    .catch((error) => setMessage(String(error)));
                }}
              >
                Link as duplicate
              </Button>
            </div>
          ))}
        </details>
      ) : null}
      {message ? (
        <p role="status" className="text-sm">
          {message}
        </p>
      ) : null}
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
          {message ? (
            <p role="status" className="text-sm">
              {message}
            </p>
          ) : null}
          <DialogHeader>
            <DialogTitle>Agent handoff</DialogTitle>
            <DialogDescription>
              The current scope, decisions, dependencies and execution evidence
              from this workspace. Source conversations remain links unless
              their content is already in the issue.
            </DialogDescription>
          </DialogHeader>
          <label className="flex gap-2 text-sm">
            <input
              type="checkbox"
              disabled={!lastAttempt}
              checked={delta}
              onChange={(event) => setDelta(event.target.checked)}
            />
            Include changes since the latest attempt
          </label>
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap rounded bg-muted/40 p-3 text-xs">
            {brief}
          </pre>
          <Button
            onClick={() => {
              void navigator.clipboard
                .writeText(brief)
                .then(() => setMessage("Agent handoff copied."))
                .catch((error) =>
                  setMessage(`Could not copy: ${String(error)}`),
                );
            }}
          >
            Copy handoff
          </Button>
        </DialogContent>
      </Dialog>
    </section>
  );
}
