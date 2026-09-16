import { dispatchMetrics, durationLabel, medianDuration } from "../metrics";
import type { DispatchContext } from "../types";
import { Button } from "@/shared/ui/button";
import { issueAlias } from "../issueEditing";
export function DispatchInsights({
  context,
  onOpenIssue,
}: {
  context: DispatchContext;
  onOpenIssue: (id: string) => void;
}) {
  const value = dispatchMetrics(context);
  const cards = [
    ["Current triage wait", value.queueAges.triage],
    ["Current review wait", value.queueAges.in_review],
    ["Completed triage intervals", value.completedDurations.triage],
    ["Completed review intervals", value.completedDurations.in_review],
    ["Observed blocked intervals", value.blockedDurations],
  ] as const;
  const backlog = context.issues
    .filter((issue) => ["triage", "backlog"].includes(issue.status))
    .sort((a, b) => a.enteredStatusAt - b.enteredStatusAt)
    .slice(0, 20);
  return (
    <div className="overflow-y-auto p-5" data-testid="dispatch-insights">
      <h2 className="text-lg font-semibold">Flow and attention</h2>
      <p className="mt-2 max-w-3xl text-sm text-muted-foreground">
        Durations use observed Buzz transitions. Imported dates and missing
        history do not count as time spent working or waiting. These measures
        describe the workflow, not a person’s productivity.
      </p>
      <div className="mt-5 grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
        {cards.map(([label, observations]) => (
          <article key={label} className="rounded-xl border border-border p-4">
            <h3 className="text-sm text-muted-foreground">{label}</h3>
            <p className="mt-2 text-2xl font-semibold">
              {durationLabel(medianDuration(observations))}
            </p>
            <p className="mt-2 text-xs text-muted-foreground">
              Median · {observations.length} observed interval(s)
            </p>
          </article>
        ))}
      </div>
      <div className="mt-5 grid gap-4 md:grid-cols-2">
        <section className="rounded-xl border border-border p-4">
          <h3 className="font-medium">History coverage</h3>
          <dl className="mt-3 grid grid-cols-2 gap-2 text-sm">
            <dt>Created in Buzz</dt>
            <dd>{value.nativeIssues}</dd>
            <dt>Imported issues</dt>
            <dd>{value.importedIssues}</dd>
            <dt>Queue entries without a measured start</dt>
            <dd>{value.missingStatusHistory}</dd>
            <dt>Blocked attempts without a measured start</dt>
            <dd>{value.blockedMissing}</dd>
            <dt>Observed reopen transitions</dt>
            <dd>{value.reopens}</dd>
          </dl>
          {context.sectionErrors.tasks ? (
            <p role="status" className="mt-3 text-sm text-warning">
              Execution history is incomplete: {context.sectionErrors.tasks}
            </p>
          ) : null}
        </section>
        <section className="rounded-xl border border-border p-4">
          <h3 className="font-medium">Current execution outcomes</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            Each attempt is counted once at its latest accepted state.
          </p>
          <dl className="mt-3 grid grid-cols-2 gap-2 text-sm">
            {Object.entries({
              assigned: "Assigned",
              picked_up: "Working",
              blocked: "Blocked",
              done: "Reported complete",
              verified: "Independently verified",
              failed: "Failed",
            }).map(([status, label]) => (
              <div key={status} className="contents">
                <dt>{label}</dt>
                <dd>{value.attempts[status]}</dd>
              </div>
            ))}
          </dl>
        </section>
      </div>
      <section className="mt-6">
        <h3 className="font-medium">Backlog review</h3>
        <p className="mt-1 text-sm text-muted-foreground">
          Review scope, ownership or relevance. Age alone does not mean an issue
          is neglected; imported work may be intentionally parked.
        </p>
        <div className="mt-3 divide-y divide-border">
          {backlog.map((issue) => (
            <div
              key={issue.issueId}
              className="flex flex-wrap items-center justify-between gap-2 py-2"
            >
              <Button
                size="sm"
                variant="link"
                className="h-auto whitespace-normal text-left"
                onClick={() => onOpenIssue(issue.issueId)}
              >
                {issueAlias(issue, context.issues)} · {issue.title}
              </Button>
              <span className="text-xs text-muted-foreground">
                {context.activity.some(
                  (entry) => entry.issueId === issue.issueId && entry.import,
                )
                  ? "Imported history"
                  : `Current status since ${new Date(issue.enteredStatusAt * 1000).toLocaleDateString()}`}
              </span>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}
