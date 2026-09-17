import * as React from "react";
import { ArrowRight, CheckCircle2, Sparkles } from "lucide-react";
import { Button } from "@/shared/ui/button";
import { activeRecords, type AutomationControls } from "../recordTypes";
import {
  AUTOMATION_TASKS,
  automationActivity,
  type AutomationTask,
} from "../automationPresentation";
import type { DispatchContext } from "../types";
import { DispatchCapacityEditor } from "./DispatchCapacityEditor";
import { AUTOMATION_ICONS, AutomationSetup } from "./AutomationSetup";
import { AutomationRunCard } from "./AutomationRunCard";

export function DispatchAutomationPanel({
  context,
  workspace,
  viewer,
  refresh,
  onOpenIssue,
}: {
  context: DispatchContext;
  workspace: string;
  viewer: string;
  refresh: () => Promise<DispatchContext>;
  onOpenIssue: (id: string) => void;
}) {
  const services = activeRecords(context.records, "automation");
  const activity = automationActivity(context);
  const [editing, setEditing] = React.useState<{
    initial?: AutomationControls;
    preset?: AutomationTask;
  } | null>(null);
  const [saved, setSaved] = React.useState(false);
  const editable =
    context.viewerRole === "maintainer" && !context.sectionErrors.records;
  return (
    <div
      className="mx-auto w-full max-w-5xl space-y-7 p-4 sm:p-6"
      data-testid="dispatch-automation"
    >
      <header className="rounded-2xl border border-border bg-card p-5 sm:p-6">
        <div className="mb-3 flex items-center gap-2 text-sm font-medium text-primary">
          <Sparkles className="size-4" aria-hidden="true" />A little less
          follow-up
        </div>
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="max-w-2xl">
            <h1 className="text-2xl font-semibold tracking-tight">
              Let Buzz handle the routine
            </h1>
            <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
              Automation is a simple rule: when something happens, Buzz takes a
              step for you. Save a message as work, remind your team, or send a
              ready issue to an agent.
            </p>
          </div>
          <Button
            disabled={!editable}
            onClick={() => {
              setSaved(false);
              setEditing({});
            }}
          >
            Set up automation
            <ArrowRight className="size-4" aria-hidden="true" />
          </Button>
        </div>
        <ol
          className="mt-5 grid gap-3 border-t border-border/60 pt-4 text-sm sm:grid-cols-3"
          aria-label="How automation works"
        >
          <li>
            <span className="font-medium">1. Choose a job</span>
            <p className="mt-1 text-muted-foreground">
              Tell Buzz what would help.
            </p>
          </li>
          <li>
            <span className="font-medium">2. Pick the work and a limit</span>
            <p className="mt-1 text-muted-foreground">
              Start small. Keep control.
            </p>
          </li>
          <li>
            <span className="font-medium">3. Review, then turn it on</span>
            <p className="mt-1 text-muted-foreground">
              See what happens. Pause anytime.
            </p>
          </li>
        </ol>
      </header>
      {context.sectionErrors.records ? (
        <div
          role="alert"
          className="rounded-lg border border-destructive/40 p-4 text-sm"
        >
          <p>
            Buzz could not load the automation settings. Refresh Dispatch to try
            again.
          </p>
          <details className="mt-2 text-xs text-muted-foreground">
            <summary>Show details</summary>
            {context.sectionErrors.records}
          </details>
        </div>
      ) : null}
      {context.viewerRole !== "maintainer" ? (
        <p className="text-sm text-muted-foreground">
          You can see what automation does here. Ask a workspace manager to set
          it up or change it.
        </p>
      ) : null}
      <section aria-labelledby="automation-ideas-heading">
        <h2
          id="automation-ideas-heading"
          className="mb-3 text-base font-semibold"
        >
          What would save you a step?
        </h2>
        <div className="grid gap-3 md:grid-cols-3">
          {AUTOMATION_TASKS.map((task) => {
            const Icon = AUTOMATION_ICONS[task.id];
            return (
              <button
                key={task.id}
                type="button"
                disabled={!editable}
                onClick={() => {
                  setSaved(false);
                  setEditing({ preset: task.id });
                }}
                className="group flex h-full flex-col items-start rounded-xl border border-border bg-card p-4 text-left transition-colors hover:border-primary/50 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
              >
                <Icon className="mb-3 size-5 text-primary" aria-hidden="true" />
                <span className="text-sm font-semibold">{task.title}</span>
                <span className="mt-2 flex-1 text-sm leading-relaxed text-muted-foreground">
                  {task.description}
                </span>
                <span className="mt-4 inline-flex items-center gap-1 text-xs font-medium text-primary">
                  Set this up
                  <ArrowRight className="size-3.5" aria-hidden="true" />
                </span>
              </button>
            );
          })}
        </div>
      </section>
      <section
        className="space-y-3"
        aria-labelledby="automation-current-heading"
      >
        <h2 id="automation-current-heading" className="text-base font-semibold">
          Your automation
        </h2>
        {saved ? (
          <p
            role="status"
            className="flex items-center gap-2 rounded-lg bg-primary/10 p-3 text-sm"
          >
            <CheckCircle2 className="size-4 shrink-0" aria-hidden="true" />
            Setup saved. Buzz is checking it. Nothing runs until you choose
            “Turn on”.
          </p>
        ) : null}
        {!services.length ? (
          <div className="rounded-xl border border-dashed border-border p-5">
            <p className="text-sm font-medium">Nothing is running yet</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Pick one of the jobs above to see how it works. You will review
              the choices before Buzz does anything.
            </p>
          </div>
        ) : (
          services.map((value) => (
            <AutomationRunCard
              key={value.record.record_id}
              value={value}
              context={context}
              workspace={workspace}
              refresh={refresh}
              onEdit={() => {
                setSaved(false);
                setEditing({ initial: value.record.payload.controls });
              }}
              onOpenIssue={onOpenIssue}
            />
          ))
        )}
      </section>
      <section
        className="rounded-xl border border-border p-5"
        aria-labelledby="automation-activity-heading"
      >
        <h2
          id="automation-activity-heading"
          className="text-base font-semibold"
        >
          What Buzz has done
        </h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Saved issues, reminders, and assignments appear here. Sending work to
          an agent does not mean the work is finished.
        </p>
        {activity.length ? (
          <ul className="mt-3 divide-y divide-border/60">
            {activity.map((entry) => (
              <li
                key={entry.eventId}
                className="flex flex-wrap items-start justify-between gap-3 py-3"
              >
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium">{entry.result}</p>
                  <p className="mt-1 break-words text-sm text-muted-foreground">
                    {entry.title}
                  </p>
                  <time
                    className="mt-1 block text-xs text-muted-foreground"
                    dateTime={new Date(entry.createdAt * 1000).toISOString()}
                  >
                    {new Date(entry.createdAt * 1000).toLocaleString()}
                  </time>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => onOpenIssue(entry.issueId)}
                >
                  View issue
                </Button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="mt-4 rounded-lg bg-muted/30 p-4 text-sm text-muted-foreground">
            No activity yet. Once you turn on a setup, check here to see what
            Buzz did.
          </p>
        )}
      </section>
      <details className="rounded-xl border border-border p-4">
        <summary className="cursor-pointer text-sm font-medium">
          Agents and work limits
        </summary>
        <p className="my-3 text-sm text-muted-foreground">
          Only needed when you want Buzz to send work to agents. Choose the
          labels each agent can handle and how much work it can take at once.
        </p>
        <DispatchCapacityEditor
          context={context}
          workspace={workspace}
          refresh={refresh}
        />
      </details>
      {editing ? (
        <AutomationSetup
          key={`${workspace}:${viewer}`}
          context={context}
          workspace={workspace}
          initial={editing.initial}
          preset={editing.preset}
          refresh={refresh}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            setSaved(true);
          }}
        />
      ) : null}
    </div>
  );
}
