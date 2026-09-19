import {
  DispatchReferenceLink,
  isDispatchResourceUrl,
} from "./DispatchReferenceLink";
import * as React from "react";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { Markdown } from "@/shared/ui/markdown";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { activeRecords, type PayloadOf, type RecordOf } from "../recordTypes";
import { publishPlanningRecord } from "../recordMutations";
import { issueAlias } from "../issueEditing";
import type { DispatchContext, DispatchProject } from "../types";
import { AutopilotCard } from "./AutopilotCard";
import { AssigneePickerDialog } from "./AssigneePickerDialog";

export function DispatchProjectsPanel({
  context,
  workspace,
  viewer,
  selected,
  onSelect,
  onOpenIssue,
  refresh,
}: {
  context: DispatchContext;
  workspace: string;
  viewer: string;
  selected: string | null;
  onSelect: (id: string | null) => void;
  onOpenIssue: (id: string) => void;
  refresh: () => Promise<DispatchContext>;
}) {
  const [query, setQuery] = React.useState("");
  const project = context.projects.find(
    (value) => value.projectId === selected,
  );
  const plans = activeRecords(context.records, "project_plan");
  if (project)
    return (
      <ProjectHome
        key={project.projectId}
        project={project}
        context={context}
        workspace={workspace}
        viewer={viewer}
        plan={plans.find(
          (value) => value.record.payload.project_id === project.projectId,
        )}
        onBack={() => onSelect(null)}
        onOpenIssue={onOpenIssue}
        refresh={refresh}
      />
    );
  return (
    <div className="overflow-y-auto p-5" data-testid="dispatch-projects">
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-lg font-semibold">Project plans</h2>
        <Input
          aria-label="Find a project"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Find a project…"
          className="max-w-xs"
        />
      </div>
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {context.projects
          .filter((project) =>
            project.name.toLowerCase().includes(query.toLowerCase()),
          )
          .map((project) => {
            const plan = plans.find(
              (value) => value.record.payload.project_id === project.projectId,
            )?.record.payload;
            const issues = context.issues.filter(
              (issue) => issue.projectId === project.projectId,
            );
            const done = issues.filter(
              (issue) => issue.status === "done",
            ).length;
            return (
              <button
                type="button"
                key={project.projectId}
                className="rounded-xl border border-border bg-card p-4 text-left transition-colors hover:bg-muted/30 focus-visible:ring-2 focus-visible:ring-ring"
                onClick={() => onSelect(project.projectId)}
              >
                <h3 className="font-medium">{project.name}</h3>
                <p className="mt-2 line-clamp-3 text-sm text-muted-foreground">
                  {plan?.outcome ||
                    "Add a clear outcome and lightweight milestones."}
                </p>
                <p className="mt-4 text-xs text-muted-foreground">
                  {issues.length} issues · {done} delivered
                  {plan?.target_date ? ` · Target ${plan.target_date}` : ""}
                </p>
              </button>
            );
          })}
      </div>
      {!context.projects.length ? (
        <p className="mt-4 text-sm text-muted-foreground">
          No projects are available in this workspace yet.
        </p>
      ) : null}
    </div>
  );
}
function ProjectHome({
  project,
  context,
  workspace,
  viewer,
  plan,
  onBack,
  onOpenIssue,
  refresh,
}: {
  project: DispatchProject;
  context: DispatchContext;
  workspace: string;
  viewer: string;
  plan: RecordOf<"project_plan"> | undefined;
  onBack: () => void;
  onOpenIssue: (id: string) => void;
  refresh: () => Promise<DispatchContext>;
}) {
  const empty: PayloadOf<"project_plan"> = {
    type: "project_plan",
    project_id: project.projectId,
    owner: viewer,
    purpose: "",
    outcome: "",
    target_date: null,
    milestones: [],
    links: [],
  };
  const [base, setBase] = React.useState(plan);
  const [draft, setDraft] = React.useState(plan?.record.payload ?? empty);
  const [open, setOpen] = React.useState(false);
  const [picker, setPicker] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [link, setLink] = React.useState("");
  const issues = context.issues.filter(
    (issue) => issue.projectId === project.projectId,
  );
  const decisions = activeRecords(context.records, "decision").filter((value) =>
    issues.some((issue) => issue.issueId === value.record.payload.issue_id),
  );
  const profiles = useUsersBatchQuery([
    ...new Set(
      [draft.owner, plan?.record.payload.owner].filter(
        (value): value is string => Boolean(value),
      ),
    ),
  ]);
  const canEdit =
    context.viewerRole !== "none" &&
    (!plan || plan.creator === viewer || context.viewerRole === "maintainer") &&
    !context.sectionErrors.records;
  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await publishPlanningRecord({
        workspace,
        previous: base,
        payload: draft,
        refresh,
      });
      setOpen(false);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };
  const value = plan?.record.payload;
  return (
    <div className="overflow-y-auto p-5" data-testid="dispatch-project-home">
      <Button variant="ghost" size="sm" onClick={onBack}>
        ← All project plans
      </Button>
      <div className="mt-3 flex items-start justify-between gap-3">
        <div>
          <h2 className="text-2xl font-semibold">{project.name}</h2>
          {value?.owner ? (
            <p className="mt-1 text-sm text-muted-foreground">
              Owner ·{" "}
              {resolveUserLabel({
                pubkey: value.owner,
                profiles: profiles.data?.profiles,
              })}
            </p>
          ) : null}
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={!canEdit}
          onClick={() => {
            setBase(plan);
            setDraft(plan?.record.payload ?? empty);
            setOpen(true);
          }}
        >
          Edit project plan
        </Button>
      </div>
      <AutopilotCard
        context={context}
        workspace={workspace}
        project={project.projectId}
        refresh={refresh}
        onOpenIssue={onOpenIssue}
      />
      <section className="mt-5 rounded-xl border border-border p-4">
        <h3 className="text-sm font-semibold">Target outcome</h3>
        <p className="mt-2 text-sm">
          {value?.outcome || "No outcome recorded yet."}
        </p>
        {value?.target_date ? (
          <p className="mt-2 text-xs text-muted-foreground">
            Target date · {value.target_date}
          </p>
        ) : null}
        {value?.purpose ? (
          <div className="mt-4 text-sm">
            <Markdown content={value.purpose} />
          </div>
        ) : null}
      </section>
      <div className="mt-5 grid gap-5 xl:grid-cols-2">
        <section>
          <h3 className="text-sm font-semibold">Milestones</h3>
          {!value?.milestones.length ? (
            <p className="mt-2 text-sm text-muted-foreground">
              Use milestones for observable outcomes, with only the issues that
              contribute to them.
            </p>
          ) : (
            value.milestones.map((milestone) => {
              const linked = context.issues.filter((issue) =>
                milestone.issue_ids.includes(issue.issueId),
              );
              const complete = linked.filter(
                (issue) => issue.status === "done",
              ).length;
              return (
                <article
                  key={milestone.id}
                  className="mt-3 rounded-lg border border-border p-3"
                >
                  <h4 className="text-sm font-medium">{milestone.title}</h4>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {complete} of {linked.length} linked issues delivered
                    {milestone.target_date ? ` · ${milestone.target_date}` : ""}
                  </p>
                  <progress
                    className="mt-2 h-2 w-full"
                    max={Math.max(1, linked.length)}
                    value={complete}
                    aria-label={`${milestone.title} progress`}
                  />
                  {linked.map((issue) => (
                    <button
                      type="button"
                      key={issue.issueId}
                      className="mt-2 block text-left text-sm text-primary hover:underline"
                      onClick={() => onOpenIssue(issue.issueId)}
                    >
                      {issueAlias(issue, context.issues)} · {issue.title}
                    </button>
                  ))}
                </article>
              );
            })
          )}
        </section>
        <section>
          <h3 className="text-sm font-semibold">Decisions and useful links</h3>
          {decisions.map((decision) => (
            <button
              type="button"
              key={decision.record.record_id}
              className="mt-2 block w-full rounded border border-border p-3 text-left text-sm hover:bg-muted/30"
              onClick={() => onOpenIssue(decision.record.payload.issue_id)}
            >
              {decision.record.payload.question}
              <span className="ml-2 text-xs text-muted-foreground">
                {decision.record.payload.selected_option
                  ? "Decided"
                  : "Awaiting decision"}
              </span>
            </button>
          ))}
          {value?.links.map((ref) => (
            <div key={`${ref.type}:${ref.ref}`} className="mt-2">
              <DispatchReferenceLink value={ref} />
            </div>
          ))}
        </section>
      </div>
      <section className="mt-5">
        <h3 className="text-sm font-semibold">
          Project work · {issues.length}
        </h3>
        <div className="mt-2 divide-y divide-border">
          {issues.map((issue) => (
            <button
              type="button"
              key={issue.issueId}
              className="flex w-full flex-wrap items-center gap-3 py-3 text-left text-sm hover:bg-muted/30"
              onClick={() => onOpenIssue(issue.issueId)}
            >
              <span className="w-24 text-xs text-muted-foreground">
                {issueAlias(issue, context.issues)}
              </span>
              <span className="flex-1">{issue.title}</span>
              <span className="text-xs text-muted-foreground">
                {issue.status.replaceAll("_", " ")}
              </span>
            </button>
          ))}
        </div>
      </section>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Edit {project.name}</DialogTitle>
            <DialogDescription>
              Connect purpose, outcomes, milestones and reference material.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={(event) => void save(event)}>
            <fieldset disabled={busy} className="space-y-4">
              <label htmlFor="project-outcome" className="grid gap-1 text-sm">
                Target outcome
                <Textarea
                  id="project-outcome"
                  value={draft.outcome}
                  required
                  maxLength={4000}
                  onChange={(event) =>
                    setDraft({ ...draft, outcome: event.target.value })
                  }
                />
              </label>
              <label htmlFor="project-purpose" className="grid gap-1 text-sm">
                Purpose and context
                <Textarea
                  id="project-purpose"
                  value={draft.purpose}
                  rows={4}
                  maxLength={16000}
                  onChange={(event) =>
                    setDraft({ ...draft, purpose: event.target.value })
                  }
                />
              </label>
              <div className="grid gap-3 sm:grid-cols-2">
                <label htmlFor="project-target" className="grid gap-1 text-sm">
                  Target date
                  <Input
                    id="project-target"
                    type="date"
                    value={draft.target_date ?? ""}
                    onChange={(event) =>
                      setDraft({
                        ...draft,
                        target_date: event.target.value || null,
                      })
                    }
                  />
                </label>
                <div className="grid gap-1 text-sm">
                  <span>Owner</span>
                  <Button
                    variant="outline"
                    type="button"
                    onClick={() => setPicker(true)}
                  >
                    {draft.owner
                      ? resolveUserLabel({
                          pubkey: draft.owner,
                          profiles: profiles.data?.profiles,
                        })
                      : "Choose owner"}
                  </Button>
                </div>
              </div>
              <AssigneePickerDialog
                open={picker}
                onOpenChange={setPicker}
                assignee={draft.owner}
                disabled={busy}
                onAssign={(owner) => {
                  setDraft({ ...draft, owner });
                  setPicker(false);
                }}
              />
              <fieldset className="space-y-3">
                <legend className="mb-2 text-sm font-medium">Milestones</legend>
                {draft.milestones.map((milestone, index) => (
                  <div
                    key={milestone.id}
                    className="rounded border border-border p-3"
                  >
                    <label
                      htmlFor={`milestone-${milestone.id}`}
                      className="grid gap-1 text-sm"
                    >
                      Milestone {index + 1}
                      <Input
                        id={`milestone-${milestone.id}`}
                        required
                        value={milestone.title}
                        maxLength={256}
                        onChange={(event) =>
                          setDraft({
                            ...draft,
                            milestones: draft.milestones.map((value) =>
                              value.id === milestone.id
                                ? { ...value, title: event.target.value }
                                : value,
                            ),
                          })
                        }
                      />
                    </label>
                    <label
                      htmlFor={`target-${milestone.id}`}
                      className="mt-2 grid gap-1 text-sm"
                    >
                      Target date
                      <Input
                        id={`target-${milestone.id}`}
                        type="date"
                        value={milestone.target_date ?? ""}
                        onChange={(event) =>
                          setDraft({
                            ...draft,
                            milestones: draft.milestones.map((value) =>
                              value.id === milestone.id
                                ? {
                                    ...value,
                                    target_date: event.target.value || null,
                                  }
                                : value,
                            ),
                          })
                        }
                      />
                    </label>
                    <details className="mt-2 text-sm">
                      <summary>
                        Contributing issues · {milestone.issue_ids.length}
                      </summary>
                      <div className="mt-2 max-h-40 overflow-y-auto">
                        {issues.map((issue) => (
                          <label
                            className="flex gap-2 py-1 text-xs"
                            key={issue.issueId}
                          >
                            <input
                              type="checkbox"
                              checked={milestone.issue_ids.includes(
                                issue.issueId,
                              )}
                              onChange={(event) =>
                                setDraft({
                                  ...draft,
                                  milestones: draft.milestones.map((value) =>
                                    value.id === milestone.id
                                      ? {
                                          ...value,
                                          issue_ids: event.target.checked
                                            ? [
                                                ...value.issue_ids,
                                                issue.issueId,
                                              ]
                                            : value.issue_ids.filter(
                                                (id) => id !== issue.issueId,
                                              ),
                                        }
                                      : value,
                                  ),
                                })
                              }
                            />
                            {issueAlias(issue, context.issues)} · {issue.title}
                          </label>
                        ))}
                      </div>
                    </details>
                    <Button
                      variant="ghost"
                      size="sm"
                      type="button"
                      onClick={() =>
                        setDraft({
                          ...draft,
                          milestones: draft.milestones.filter(
                            (value) => value.id !== milestone.id,
                          ),
                        })
                      }
                    >
                      Remove milestone
                    </Button>
                  </div>
                ))}
                <Button
                  variant="outline"
                  size="sm"
                  type="button"
                  disabled={draft.milestones.length >= 50}
                  onClick={() =>
                    setDraft({
                      ...draft,
                      milestones: [
                        ...draft.milestones,
                        {
                          id: crypto.randomUUID(),
                          title: "",
                          target_date: null,
                          issue_ids: [],
                        },
                      ],
                    })
                  }
                >
                  Add milestone
                </Button>
              </fieldset>
              <fieldset>
                <legend className="mb-2 text-sm font-medium">
                  Conversations, documents and resources
                </legend>
                {draft.links.map((ref) => (
                  <div className="mb-2 flex items-center gap-2" key={ref.ref}>
                    <span className="min-w-0 flex-1 break-all text-xs">
                      {ref.ref}
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      type="button"
                      onClick={() =>
                        setDraft({
                          ...draft,
                          links: draft.links.filter(
                            (value) => value.ref !== ref.ref,
                          ),
                        })
                      }
                    >
                      Remove
                    </Button>
                  </div>
                ))}
                <div className="flex gap-2">
                  <Input
                    aria-label="Project resource URL"
                    placeholder="Paste a conversation or document link"
                    value={link}
                    onChange={(event) => setLink(event.target.value)}
                  />
                  <Button
                    variant="outline"
                    type="button"
                    disabled={
                      !isDispatchResourceUrl(link) || draft.links.length >= 50
                    }
                    onClick={() => {
                      setDraft({
                        ...draft,
                        links: [
                          ...draft.links,
                          { type: "url", ref: link.trim() },
                        ],
                      });
                      setLink("");
                    }}
                  >
                    Add
                  </Button>
                </div>
              </fieldset>
              {error ? (
                <p role="alert" className="text-sm text-destructive">
                  {error}
                </p>
              ) : null}
              <Button type="submit" disabled={busy || !draft.outcome.trim()}>
                {busy ? "Saving…" : "Save project plan"}
              </Button>
            </fieldset>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
