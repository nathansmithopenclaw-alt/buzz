import * as React from "react";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  activeRecords,
  type DecisionOption,
  type RecordOf,
} from "../recordTypes";
import { publishPlanningRecord } from "../recordMutations";
import type { DispatchContext, DispatchIssue } from "../types";
import { AssigneePickerDialog } from "./AssigneePickerDialog";

export function DecisionPanel({
  issue,
  context,
  viewer,
  refresh,
}: {
  issue: DispatchIssue;
  context: DispatchContext;
  viewer: string;
  refresh: () => Promise<DispatchContext>;
}) {
  const [open, setOpen] = React.useState(false);
  const [question, setQuestion] = React.useState("");
  const [details, setDetails] = React.useState("");
  const [decider, setDecider] = React.useState<string | null>(viewer);
  const [picker, setPicker] = React.useState(false);
  const [options, setOptions] = React.useState<DecisionOption[]>([
    { id: crypto.randomUUID(), label: "Proceed", consequence: "" },
    { id: crypto.randomUUID(), label: "Hold", consequence: "" },
  ]);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const decisions = activeRecords(context.records, "decision").filter(
    (value) => value.record.payload.issue_id === issue.issueId,
  );
  const profiles = useUsersBatchQuery([
    ...new Set([
      ...(decider ? [decider] : []),
      ...decisions.flatMap((value) => [
        value.record.payload.decider,
        value.author,
      ]),
    ]),
  ]);
  const label = (pubkey: string) =>
    resolveUserLabel({ pubkey, profiles: profiles.data?.profiles });
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!decider) return;
    setBusy(true);
    setError(null);
    try {
      await publishPlanningRecord({
        workspace: issue.streamId,
        payload: {
          type: "decision",
          issue_id: issue.issueId,
          decider,
          question: question.trim(),
          context: details,
          options,
          evidence: issue.externalRefs
            .filter((ref) =>
              ["url", "doc", "file", "pr", "pull_request"].includes(ref.type),
            )
            .slice(0, 50),
          selected_option: null,
          rationale: null,
        },
        refresh,
      });
      setOpen(false);
      setQuestion("");
      setDetails("");
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section
      className="mt-5 border-t border-border pt-5"
      aria-label="Decisions"
    >
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-sm font-semibold">Decisions</h2>
        <Button
          variant="ghost"
          size="sm"
          disabled={
            context.viewerRole === "none" ||
            Boolean(context.sectionErrors.records)
          }
          onClick={() => setOpen(true)}
        >
          Ask for a decision
        </Button>
      </div>
      {context.sectionErrors.records ? (
        <p role="status" className="mt-2 text-sm text-muted-foreground">
          Shared decisions are unavailable: {context.sectionErrors.records}
        </p>
      ) : null}
      {!decisions.length ? (
        <p className="mt-2 text-sm text-muted-foreground">
          Record a specific question when this work needs a person’s judgment.
        </p>
      ) : null}
      <div className="mt-3 space-y-3">
        {decisions.map((value) => (
          <DecisionCard
            key={value.record.record_id}
            value={value}
            workspace={issue.streamId}
            viewer={viewer}
            label={label}
            refresh={refresh}
          />
        ))}
      </div>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Ask for a decision</DialogTitle>
            <DialogDescription>
              Give one person the context, options and consequences needed to
              move this issue forward.
            </DialogDescription>
          </DialogHeader>
          <form className="space-y-3" onSubmit={(event) => void submit(event)}>
            <fieldset disabled={busy} className="space-y-3">
              <label className="grid gap-1 text-sm" htmlFor="dispatch-question">
                Question
                <Input
                  id="dispatch-question"
                  value={question}
                  maxLength={256}
                  required
                  onChange={(event) => setQuestion(event.target.value)}
                  placeholder="What needs to be decided?"
                />
              </label>
              <div>
                <span className="mb-1 block text-sm">Decision maker</span>
                <Button
                  variant="outline"
                  type="button"
                  onClick={() => setPicker(true)}
                >
                  {decider ? label(decider) : "Choose a person"}
                </Button>
              </div>
              <AssigneePickerDialog
                open={picker}
                onOpenChange={setPicker}
                assignee={decider}
                disabled={busy}
                onAssign={(value) => {
                  setDecider(value);
                  setPicker(false);
                }}
              />
              <label
                htmlFor="dispatch-decision-context"
                className="grid gap-1 text-sm"
              >
                Context
                <Textarea
                  id="dispatch-decision-context"
                  rows={4}
                  value={details}
                  maxLength={16000}
                  onChange={(event) => setDetails(event.target.value)}
                />
              </label>
              <fieldset className="space-y-3">
                <legend className="mb-2 text-sm font-medium">
                  Options and consequences
                </legend>
                {options.map((option, index) => (
                  <div
                    key={option.id}
                    className="rounded border border-border p-3"
                  >
                    <label
                      htmlFor={`option-${option.id}`}
                      className="grid gap-1 text-sm"
                    >
                      Option {index + 1}
                      <Input
                        id={`option-${option.id}`}
                        value={option.label}
                        required
                        maxLength={120}
                        onChange={(event) =>
                          setOptions((current) =>
                            current.map((value) =>
                              value.id === option.id
                                ? { ...value, label: event.target.value }
                                : value,
                            ),
                          )
                        }
                      />
                    </label>
                    <div className="mt-2 grid gap-1 text-sm">
                      <label htmlFor={`consequence-${option.id}`}>
                        Consequence
                      </label>
                      <Textarea
                        id={`consequence-${option.id}`}
                        value={option.consequence}
                        rows={2}
                        maxLength={2000}
                        onChange={(event) =>
                          setOptions((current) =>
                            current.map((value) =>
                              value.id === option.id
                                ? { ...value, consequence: event.target.value }
                                : value,
                            ),
                          )
                        }
                      />
                    </div>
                    {options.length > 2 ? (
                      <Button
                        size="sm"
                        variant="ghost"
                        type="button"
                        onClick={() =>
                          setOptions((current) =>
                            current.filter((value) => value.id !== option.id),
                          )
                        }
                      >
                        Remove option
                      </Button>
                    ) : null}
                  </div>
                ))}
                <Button
                  variant="outline"
                  size="sm"
                  type="button"
                  disabled={options.length >= 8}
                  onClick={() =>
                    setOptions((current) => [
                      ...current,
                      { id: crypto.randomUUID(), label: "", consequence: "" },
                    ])
                  }
                >
                  Add option
                </Button>
              </fieldset>
              {error ? (
                <p role="alert" className="text-sm text-destructive">
                  {error}
                </p>
              ) : null}
              <Button
                type="submit"
                disabled={!decider || !question.trim() || busy}
              >
                {busy ? "Saving…" : "Create decision"}
              </Button>
            </fieldset>
          </form>
        </DialogContent>
      </Dialog>
    </section>
  );
}
function DecisionCard({
  value,
  workspace,
  viewer,
  label,
  refresh,
}: {
  value: RecordOf<"decision">;
  workspace: string;
  viewer: string;
  label: (key: string) => string;
  refresh: () => Promise<DispatchContext>;
}) {
  const decision = value.record.payload;
  const resolved = decision.options.find(
    (option) => option.id === decision.selected_option,
  );
  const [base, setBase] = React.useState(value);
  const [choice, setChoice] = React.useState("");
  const [rationale, setRationale] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const decide = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!choice || !rationale.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await publishPlanningRecord({
        workspace,
        previous: base,
        payload: {
          ...base.record.payload,
          selected_option: choice,
          rationale: rationale.trim(),
        },
        refresh,
      });
      setChoice("");
      setRationale("");
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <article className="rounded-lg border border-border bg-muted/10 p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-sm font-medium">{decision.question}</h3>
        <span className="text-xs text-muted-foreground">
          {resolved ? "Decided" : "Waiting for"} · {label(decision.decider)}
        </span>
      </div>
      <p className="mt-2 whitespace-pre-wrap text-sm">{decision.context}</p>
      {resolved ? (
        <div
          data-testid="dispatch-decision-resolution"
          className="mt-3 rounded bg-muted/50 p-3 text-sm"
        >
          <strong>{resolved.label}</strong>
          <p className="mt-1">{decision.rationale}</p>
          <span className="text-xs text-muted-foreground">
            Recorded by {label(value.author)} ·{" "}
            {new Date(value.created_at * 1000).toLocaleString()}
          </span>
        </div>
      ) : (
        <div className="mt-3 space-y-2">
          {decision.options.map((option) => (
            <div
              key={option.id}
              className="rounded border border-border p-2 text-sm"
            >
              <strong>{option.label}</strong>
              <p className="text-muted-foreground">{option.consequence}</p>
              {viewer === decision.decider ? (
                <Button
                  size="sm"
                  variant={choice === option.id ? "default" : "outline"}
                  disabled={busy}
                  onClick={() => {
                    setBase(value);
                    setChoice(option.id);
                  }}
                >
                  Choose {option.label}
                </Button>
              ) : null}
            </div>
          ))}
        </div>
      )}
      {!resolved && choice && viewer === decision.decider ? (
        <form
          className="mt-3 space-y-2"
          onSubmit={(event) => void decide(event)}
        >
          <label
            htmlFor={`rationale-${value.record.record_id}`}
            className="grid gap-1 text-sm"
          >
            Reason for this decision
            <Textarea
              id={`rationale-${value.record.record_id}`}
              value={rationale}
              required
              maxLength={4000}
              onChange={(event) => setRationale(event.target.value)}
            />
          </label>
          {base.event_id !== value.event_id ? (
            <p role="status" className="text-sm text-warning">
              The decision changed. Review the current options and choose again;
              your rationale is retained.
            </p>
          ) : null}
          <Button
            type="submit"
            disabled={
              busy || !rationale.trim() || base.event_id !== value.event_id
            }
          >
            {busy ? "Recording…" : "Record decision"}
          </Button>
        </form>
      ) : null}
      {decision.evidence.length ? (
        <details className="mt-2 text-xs">
          <summary>Supporting evidence · {decision.evidence.length}</summary>
          {decision.evidence.map((ref) => (
            <p key={`${ref.type}:${ref.ref}`} className="mt-1 break-all">
              {ref.type}:{" "}
              {/^https?:\/\//i.test(ref.ref) ? (
                <a
                  className="underline"
                  href={ref.ref}
                  target="_blank"
                  rel="noreferrer"
                >
                  {ref.ref}
                </a>
              ) : (
                ref.ref
              )}
            </p>
          ))}
        </details>
      ) : null}
      {error ? (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </article>
  );
}
