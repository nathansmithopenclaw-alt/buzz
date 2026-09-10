import * as React from "react";
import { Button } from "@/shared/ui/button";
import {
  conflictTitle,
  proposedConflictFields,
  unresolvedDiagnostics,
} from "../conflicts";
import type { DispatchDiagnostic, DispatchIssue } from "../types";
import type { DispatchIssueEdit } from "../issueEditing";
import type { DispatchIssueMutation } from "../mutations";

export function IssueConflictPanel({
  issue,
  diagnostics,
  disabled,
  onSave,
}: {
  issue: DispatchIssue;
  diagnostics: DispatchDiagnostic[];
  disabled: boolean;
  onSave: (mutation: DispatchIssueMutation) => Promise<DispatchIssue>;
}) {
  const conflicts = unresolvedDiagnostics(diagnostics, [issue]).filter(
    (value) => value.issueId === issue.issueId,
  );
  if (!conflicts.length) return null;
  return (
    <section
      aria-label="Issue conflicts"
      className="mt-4 space-y-3 rounded-lg border border-warning/40 bg-warning-bg p-3"
    >
      <h2 className="text-sm font-semibold">Changes needing review</h2>
      {conflicts.map((diagnostic, index) => (
        <Conflict
          key={diagnostic.eventId ?? index}
          issue={issue}
          diagnostic={diagnostic}
          disabled={disabled}
          onSave={onSave}
        />
      ))}
    </section>
  );
}
function Conflict({
  issue,
  diagnostic,
  disabled,
  onSave,
}: {
  issue: DispatchIssue;
  diagnostic: DispatchDiagnostic;
  disabled: boolean;
  onSave: (mutation: DispatchIssueMutation) => Promise<DispatchIssue>;
}) {
  const [base, setBase] = React.useState(issue);
  const [busy, setBusy] = React.useState(false);
  const changed = base.tipEvent !== issue.tipEvent;
  const proposed = proposedConflictFields(diagnostic, base);
  const [selected, setSelected] = React.useState<string[]>([]);
  const [error, setError] = React.useState<string | null>(null);
  const reconcile = async (keepCurrent: boolean) => {
    if (!diagnostic.eventId) return;
    if (changed) return;
    setBusy(true);
    setError(null);
    const fields: DispatchIssueEdit = Object.fromEntries(
      Object.entries(proposed).filter(([key]) => selected.includes(key)),
    );
    try {
      await onSave({
        type: "edit",
        expectedTip: base.tipEvent,
        fields: {
          ...(keepCurrent ? {} : fields),
          externalRefs: [
            ...(!keepCurrent && fields.externalRefs
              ? fields.externalRefs
              : base.externalRefs),
            { type: "resolved_conflict", ref: diagnostic.eventId },
          ],
        },
        reason: `Reconciled ${diagnostic.eventId}: ${keepCurrent ? "kept current values" : `accepted ${selected.join(", ")}; kept remaining current values`}`,
      });
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };
  const display = (value: unknown) =>
    value === null || value === undefined
      ? "Empty"
      : typeof value === "string"
        ? value
        : JSON.stringify(value);
  return (
    <details className="text-sm">
      <summary className="cursor-pointer font-medium">
        {conflictTitle[diagnostic.kind] ?? "Preserved history needs review"}
      </summary>
      <p className="mt-2 text-xs text-muted-foreground">
        Review the current value and the proposed change. Your choice creates a
        new history entry.
      </p>
      <div className="mt-2 space-y-3">
        {Object.entries(proposed).map(([field, value]) => (
          <label
            key={field}
            className="block rounded border border-border bg-background p-2"
          >
            <span className="flex gap-2 font-medium">
              <input
                type="checkbox"
                disabled={disabled || busy || changed}
                checked={selected.includes(field)}
                onChange={(event) =>
                  setSelected((current) =>
                    event.target.checked
                      ? [...current, field]
                      : current.filter((key) => key !== field),
                  )
                }
              />
              {field.replace(/([A-Z])/g, " $1")}
            </span>
            <span className="mt-1 grid gap-2 text-xs sm:grid-cols-2">
              <span className="whitespace-pre-wrap break-words">
                Current: {display(base[field as keyof DispatchIssue])}
              </span>
              <span className="whitespace-pre-wrap break-words">
                Proposed: {display(value)}
              </span>
            </span>
          </label>
        ))}
      </div>
      {changed ? (
        <div role="status" className="mt-2 text-warning">
          <p>
            The accepted issue changed while you were reviewing. Refresh the
            comparison before choosing a resolution.
          </p>
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => {
              setBase(issue);
              setSelected([]);
            }}
          >
            Review latest version
          </Button>
        </div>
      ) : null}
      {diagnostic.eventId ? (
        <div className="mt-3 flex flex-wrap gap-2">
          <Button
            size="sm"
            disabled={disabled || busy || changed || !selected.length}
            onClick={() => void reconcile(false)}
          >
            Apply selected edits
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={disabled || busy || changed}
            onClick={() => void reconcile(true)}
          >
            Keep current version
          </Button>
        </div>
      ) : null}
      {error ? (
        <p role="alert" className="mt-2 text-destructive">
          {error}
        </p>
      ) : null}
      <details className="mt-2 text-xs text-muted-foreground">
        <summary>Signed record details</summary>
        <pre className="mt-2 whitespace-pre-wrap break-all">
          {diagnostic.message}
        </pre>
      </details>
    </details>
  );
}
