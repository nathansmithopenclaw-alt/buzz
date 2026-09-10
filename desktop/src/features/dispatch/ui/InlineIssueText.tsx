import * as React from "react";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { Markdown } from "@/shared/ui/markdown";
import type { DispatchIssue } from "../types";
import type { DispatchIssueMutation } from "../mutations";

export function InlineIssueText({
  issue,
  field,
  disabled,
  onSave,
}: {
  issue: DispatchIssue;
  field: "title" | "body";
  disabled: boolean;
  onSave: (mutation: DispatchIssueMutation) => Promise<DispatchIssue>;
}) {
  const [editing, setEditing] = React.useState(false);
  const [draft, setDraft] = React.useState(issue[field]);
  const [base, setBase] = React.useState(issue.tipEvent);
  const [expanded, setExpanded] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(null);
    try {
      await onSave({
        type: "edit",
        fields: { [field]: draft },
        expectedTip: base,
      });
      setEditing(false);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  };
  const title = field === "title";
  if (!editing)
    return title ? (
      <h1
        aria-label={issue.title}
        className="text-2xl font-semibold leading-tight tracking-tight"
      >
        <button
          type="button"
          className="rounded text-left hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring"
          disabled={disabled}
          aria-label={`Edit title: ${issue.title}`}
          onClick={() => {
            setDraft(issue.title);
            setBase(issue.tipEvent);
            setEditing(true);
          }}
        >
          {issue.title}
        </button>
      </h1>
    ) : (
      <div>
        <div className={expanded ? "" : "max-h-64 overflow-hidden"}>
          <Markdown content={issue.body || "No description yet."} />
        </div>
        {issue.body.length > 500 ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? "Collapse description" : "Show full description"}
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          disabled={disabled}
          onClick={() => {
            setDraft(issue.body);
            setBase(issue.tipEvent);
            setEditing(true);
          }}
        >
          Edit description
        </Button>
      </div>
    );
  return (
    <form
      className="w-full space-y-2"
      onSubmit={(event) => void submit(event)}
      onKeyDown={(event) => {
        if (event.key === "Escape" && !disabled) setEditing(false);
        if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
          event.preventDefault();
          event.currentTarget.requestSubmit();
        }
      }}
    >
      {title ? (
        <Input
          autoFocus
          aria-label="Issue title"
          value={draft}
          maxLength={256}
          disabled={disabled}
          required
          onChange={(event) => setDraft(event.target.value)}
        />
      ) : (
        <Textarea
          autoFocus
          aria-label="Issue description"
          value={draft}
          rows={8}
          disabled={disabled}
          onChange={(event) => setDraft(event.target.value)}
        />
      )}
      {issue.tipEvent !== base ? (
        <div
          role="status"
          className="rounded border border-warning/40 p-3 text-sm"
        >
          <p>
            This issue changed while you were editing. Your draft is retained
            above.
          </p>
          <p className="mt-2 font-medium">
            Current {title ? "title" : "description"}
          </p>
          <p className="mt-1 max-h-40 overflow-y-auto whitespace-pre-wrap">
            {issue[field] || "Empty"}
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="mt-2"
            disabled={disabled}
            onClick={() => {
              setBase(issue.tipEvent);
              setError(null);
            }}
          >
            Keep my draft on this version
          </Button>
        </div>
      ) : null}
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
      <div className="flex gap-2">
        <Button
          type="submit"
          size="sm"
          disabled={disabled || issue.tipEvent !== base}
        >
          {disabled ? "Saving…" : "Save"}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={disabled}
          onClick={() => setEditing(false)}
        >
          Cancel
        </Button>
      </div>
    </form>
  );
}
