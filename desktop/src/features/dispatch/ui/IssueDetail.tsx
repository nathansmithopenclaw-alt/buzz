import { IssueWriteRecoveryPanel } from "./IssueWriteRecoveryPanel";
import { IssueReminderButton } from "./IssueReminderButton";
import { DispatchDraftsPanel } from "./DispatchDraftsPanel";
import { ConversationUpdatePanel } from "./ConversationUpdatePanel";
import { DispatchAssignmentDialog } from "./DispatchAssignmentDialog";
import { DispatchReferenceLink } from "./DispatchReferenceLink";
import { IssueActivationPanel } from "./IssueActivationPanel";
import { DeliveryPanel } from "./DeliveryPanel";
import { DecisionPanel } from "./DecisionPanel";
import { IssueContextTools } from "./IssueContextTools";
import { IssueConflictPanel } from "./IssueConflictPanel";
import { InlineIssueText } from "./InlineIssueText";
import {
  ArrowLeft,
  ExternalLink,
  FolderGit2,
  History,
  Link2,
  MessageSquare,
  Send,
  UserRound,
} from "lucide-react";
import * as React from "react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { Skeleton } from "@/shared/ui/skeleton";
import { Textarea } from "@/shared/ui/textarea";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { AssigneePickerDialog } from "./AssigneePickerDialog";
import { TaskExecutionPanel } from "./TaskExecutionPanel";
import { IssueEditor } from "./IssueEditor";
import { issueAlias } from "../issueEditing";
import { chronologicalActivity } from "../selectors.mjs";
import { assigneeLabel } from "../viewModel";
import type { DispatchIssueMutation } from "../mutations";
import type {
  DispatchContext,
  DispatchActivity,
  DispatchDiagnostic,
  DispatchComment,
  DispatchIssue,
  DispatchLinkedTask,
  DispatchPriority,
  DispatchProject,
  DispatchStatus,
} from "../types";

const STATUSES: DispatchStatus[] = [
  "triage",
  "backlog",
  "todo",
  "in_progress",
  "in_review",
  "done",
  "cancelled",
];
const PRIORITIES: DispatchPriority[] = ["P0", "P1", "P2", "P3"];
const TERMINAL = new Set<DispatchStatus>(["done", "cancelled"]);

function titleCase(value: string) {
  return value
    .replaceAll("_", " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function dateLabel(timestamp: number, withYear = false) {
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    ...(withYear ? { year: "numeric" } : {}),
  }).format(new Date(timestamp * 1_000));
}

function safeExternalHref(value: string | null | undefined) {
  if (!value) return undefined;
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:"
      ? url.toString()
      : undefined;
  } catch {
    return undefined;
  }
}

function canTransition(
  role: "none" | "member" | "maintainer",
  current: DispatchStatus,
  next: DispatchStatus,
) {
  if (role === "none") return false;
  return (
    role === "maintainer" || (!TERMINAL.has(current) && !TERMINAL.has(next))
  );
}

function ProfileAvatar({
  profiles,
  pubkey,
}: {
  profiles?: UserProfileLookup;
  pubkey: string;
}) {
  const profile = profiles?.[pubkey.toLowerCase()];
  return (
    <UserAvatar
      avatarUrl={profile?.avatarUrl ?? null}
      displayName={resolveUserLabel({ profiles, pubkey })}
      size="sm"
    />
  );
}

/**
 * Assignee row: shows who the issue is assigned to, not the raw key.
 *
 * The name comes from kind:0 profile metadata via {@link assigneeLabel} — the
 * same resolver the board and list views use — and falls back to a truncated
 * pubkey when no profile has been published. Clicking the row opens
 * {@link AssigneePickerDialog}, which is the channel members popup rendered
 * from the same shared components; setting an assignee is a search, not a hex
 * field.
 */
function AssigneeControl({
  canEdit,
  currentPubkey,
  isMutating,
  onMutate,
  profiles,
  pubkey,
}: {
  canEdit: boolean;
  currentPubkey?: string;
  isMutating: boolean;
  onMutate: (mutation: DispatchIssueMutation) => Promise<DispatchIssue>;
  profiles?: UserProfileLookup;
  pubkey: string | null;
}) {
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const assign = (next: string | null) =>
    void onMutate({ type: "assignee", assignee: next }).catch(() => undefined);

  return (
    <div className="flex items-center gap-1">
      <button
        aria-label={pubkey ? "Change issue assignee" : "Set issue assignee"}
        className="flex min-w-0 flex-1 items-center gap-1.5 rounded-md px-1 py-1 text-left hover:bg-muted/60 disabled:opacity-50 disabled:hover:bg-transparent"
        data-testid="dispatch-assignee"
        disabled={!canEdit || isMutating}
        onClick={() => setPickerOpen(true)}
        type="button"
        title={pubkey ?? undefined}
      >
        {pubkey ? <ProfileAvatar profiles={profiles} pubkey={pubkey} /> : null}
        <span
          className={cn(
            "truncate",
            pubkey ? undefined : "text-muted-foreground",
          )}
          data-testid="dispatch-assignee-name"
        >
          {pubkey ? assigneeLabel(pubkey, profiles ?? {}) : "Unassigned"}
        </span>
      </button>
      <Button
        aria-label="Assign to me"
        disabled={!canEdit || isMutating || !currentPubkey}
        onClick={() => assign(currentPubkey ?? null)}
        size="icon"
        variant="ghost"
      >
        <UserRound className="h-4 w-4" />
      </Button>
      <AssigneePickerDialog
        assignee={pubkey}
        disabled={!canEdit || isMutating}
        onAssign={assign}
        onOpenChange={setPickerOpen}
        open={pickerOpen}
        profiles={profiles}
      />
    </div>
  );
}

function Property({
  children,
  label,
}: {
  children: React.ReactNode;
  label: string;
}) {
  return (
    <div className="grid min-h-9 grid-cols-[76px_1fr] items-center gap-2 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <span className="min-w-0">{children}</span>
    </div>
  );
}

export function IssueDetailSkeleton() {
  return (
    <main
      aria-label="Loading issue"
      className="grid h-full grid-cols-1 overflow-y-auto lg:grid-cols-[minmax(0,1fr)_20rem] lg:overflow-hidden"
      role="status"
    >
      <div className="space-y-5 p-8">
        <Skeleton className="h-4 w-44" />
        <Skeleton className="h-8 w-4/5" />
        <Skeleton className="h-48 w-full" />
        <Skeleton className="h-40 w-full" />
      </div>
      <div className="space-y-4 border-l border-border p-6">
        <Skeleton className="h-4 w-24" />
        {[
          "status",
          "priority",
          "assignee",
          "project",
          "labels",
          "due",
          "updated",
        ].map((property) => (
          <Skeleton className="h-8 w-full" key={property} />
        ))}
      </div>
    </main>
  );
}

export function IssueDetailUnavailable({
  error,
  onBack,
}: {
  error: Error;
  onBack: () => void;
}) {
  return (
    <main className="flex h-full flex-col items-center justify-center p-8 text-center">
      <h1 className="text-lg font-semibold">Issue unavailable</h1>
      <p className="mt-2 max-w-lg text-sm text-muted-foreground">
        Canonical root or history data could not be loaded. Buzz will not
        silently show a stale record.
      </p>
      <p className="mt-2 max-w-lg text-xs text-destructive">{error.message}</p>
      <Button className="mt-5" onClick={onBack} variant="outline">
        <ArrowLeft className="mr-2 h-4 w-4" />
        Back to Dispatch
      </Button>
    </main>
  );
}

export function IssueDetail({
  activity,
  context,
  refresh,
  diagnostics,
  comments,
  currentPubkey,
  issue,
  issues,
  isMutating,
  onAddComment,
  onBack,
  onMutate,
  onOpenIssue,
  profiles,
  profilesError,
  projects,
  role,
  sectionErrors,
  taskLinks,
}: {
  context: DispatchContext;
  refresh: () => Promise<DispatchContext>;
  activity: readonly DispatchActivity[];
  diagnostics: DispatchDiagnostic[];
  comments: readonly DispatchComment[];
  currentPubkey?: string;
  issue: DispatchIssue;
  issues: readonly DispatchIssue[];
  isMutating: boolean;
  onAddComment: (body: string) => Promise<void>;
  onBack: () => void;
  onMutate: (mutation: DispatchIssueMutation) => Promise<DispatchIssue>;
  onOpenIssue: (issueId: string) => void;
  profiles?: UserProfileLookup;
  profilesError?: string | null;
  projects: readonly DispatchProject[];
  role: "none" | "member" | "maintainer";
  sectionErrors: {
    comments: string | null;
    projects: string | null;
    tasks: string | null;
  };
  taskLinks: readonly DispatchLinkedTask[];
}) {
  const sectionId = React.useId();
  const [editing, setEditing] = React.useState(false);
  const [comment, setComment] = React.useState("");
  const [reason, setReason] = React.useState("");
  const project = projects.find(
    (candidate) => candidate.projectId === issue.projectId,
  );
  const imported = activity.find((entry) => entry.import)?.import ?? null;
  const importedSourceUrl = safeExternalHref(imported?.source_url);
  const timeline = chronologicalActivity(
    activity,
    sectionErrors.comments ? [] : comments,
  );
  const canEdit = role !== "none";
  const relations = [
    ...issue.relations.map((relation) => ({
      label: titleCase(relation.kind),
      target: relation.targetIssueId,
    })),
    ...issues.flatMap((candidate) =>
      candidate.relations
        .filter((relation) => relation.targetIssueId === issue.issueId)
        .map((relation) => ({
          label:
            relation.kind === "blocks"
              ? "Blocked by"
              : relation.kind === "parent"
                ? "Child"
                : "Duplicated by",
          target: candidate.issueId,
        })),
    ),
  ];

  return (
    <main
      className="grid h-full min-h-0 grid-cols-1 overflow-y-auto lg:grid-cols-[minmax(0,1fr)_20rem] lg:overflow-hidden"
      data-testid="dispatch-issue-detail"
    >
      <div className="min-w-0 lg:overflow-y-auto px-4 py-5 sm:px-8 sm:py-6">
        <button
          className="flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground"
          onClick={onBack}
          data-testid="dispatch-detail-back"
          type="button"
        >
          <ArrowLeft className="h-4 w-4" />
          Dispatch <span>›</span>
          <span
            className="max-w-48 truncate text-foreground"
            title={issue.identifier}
          >
            {issueAlias(issue, issues)}
          </span>
        </button>
        <div className="mt-4 flex items-start justify-between gap-3">
          <InlineIssueText
            key={`title:${issue.issueId}`}
            issue={issue}
            field="title"
            disabled={!canEdit || isMutating}
            onSave={onMutate}
          />
          <Button
            variant="outline"
            size="sm"
            disabled={!canEdit || isMutating}
            onClick={() => setEditing(true)}
          >
            Edit issue
          </Button>
        </div>
        <div className="mt-3 flex flex-wrap items-start gap-2">
          {currentPubkey ? (
            <IssueReminderButton key={issue.rootEvent} issue={issue} />
          ) : null}
          {currentPubkey ? (
            <DispatchAssignmentDialog
              key={`assignment:${issue.issueId}`}
              issue={issue}
              context={context}
              refresh={refresh}
            />
          ) : null}
        </div>
        <nav
          aria-label="Issue sections"
          className="sticky top-0 z-10 mt-3 flex flex-wrap gap-1 border-b border-border bg-background py-2"
        >
          {[
            ["description", "Details"],
            ["execution", "Execution"],
            ["decisions", "Decisions"],
            ["delivery", "Delivery"],
            ["activity", "Activity"],
          ].map(([key, label]) => (
            <Button
              key={key}
              size="sm"
              variant="ghost"
              onClick={() => {
                const target = document.getElementById(`${sectionId}-${key}`);
                target?.scrollIntoView({ behavior: "smooth", block: "start" });
                target?.focus({ preventScroll: true });
              }}
            >
              {label}
            </Button>
          ))}
        </nav>
        <IssueActivationPanel
          key={`activation:${issue.issueId}`}
          issue={issue}
          context={context}
          refresh={refresh}
        />
        <IssueConflictPanel
          issue={issue}
          diagnostics={diagnostics}
          disabled={!canEdit || isMutating}
          onSave={onMutate}
        />
        <IssueEditor
          key={issue.issueId}
          open={editing}
          onOpenChange={setEditing}
          issue={issue}
          issues={issues}
          projects={projects}
          onSave={onMutate}
          disabled={isMutating}
        />
        <section
          id={`${sectionId}-description`}
          tabIndex={-1}
          className="mt-5 scroll-mt-16 text-sm"
        >
          <InlineIssueText
            key={`body:${issue.issueId}`}
            issue={issue}
            field="body"
            disabled={!canEdit || isMutating}
            onSave={onMutate}
          />
        </section>

        <DispatchDraftsPanel
          workspace={issue.streamId}
          snapshotAt={context.snapshotAt}
        />
        <IssueWriteRecoveryPanel
          workspace={issue.streamId}
          snapshotAt={context.snapshotAt}
          refresh={refresh}
        />
        <ConversationUpdatePanel
          issue={issue}
          context={context}
          refresh={refresh}
        />
        <div
          id={`${sectionId}-execution`}
          tabIndex={-1}
          className="scroll-mt-16"
        >
          <TaskExecutionPanel
            refresh={refresh}
            viewer={currentPubkey}
            issue={issue}
            tasks={taskLinks}
            error={sectionErrors.tasks}
            profiles={profiles}
          />
        </div>
        <div
          id={`${sectionId}-decisions`}
          tabIndex={-1}
          className="scroll-mt-16"
        >
          {currentPubkey ? (
            <DecisionPanel
              key={`decision:${issue.issueId}`}
              issue={issue}
              context={context}
              viewer={currentPubkey}
              refresh={refresh}
            />
          ) : null}
        </div>
        <div
          id={`${sectionId}-delivery`}
          tabIndex={-1}
          className="scroll-mt-16"
        >
          <DeliveryPanel
            key={`delivery:${issue.issueId}`}
            issue={issue}
            context={context}
            refresh={refresh}
          />
        </div>
        <IssueContextTools
          key={`context:${issue.issueId}`}
          context={context}
          issue={issue}
          disabled={!canEdit || isMutating}
          onOpenIssue={onOpenIssue}
          onSave={onMutate}
        />
        <section
          className="mt-5 scroll-mt-16 border-t border-border pt-5"
          id={`${sectionId}-activity`}
          tabIndex={-1}
          aria-labelledby="activity-heading"
        >
          <h2
            className="flex items-center gap-2 text-sm font-semibold"
            id="activity-heading"
          >
            <History className="h-4 w-4 text-muted-foreground" />
            Activity
          </h2>
          {profilesError ? (
            <p className="mt-3 rounded-md border border-border bg-muted/25 px-3 py-2 text-xs text-muted-foreground">
              Some profile names are unavailable: {profilesError}
            </p>
          ) : null}
          {sectionErrors.comments ? (
            <p className="mt-3 rounded-md border border-border bg-muted/25 px-3 py-2 text-xs text-muted-foreground">
              Comments are unavailable; canonical revision history remains
              visible. {sectionErrors.comments}
            </p>
          ) : null}
          <ol className="mt-3 space-y-3">
            {timeline.map((entry) => {
              const label = resolveUserLabel({
                currentPubkey,
                profiles,
                pubkey: entry.authorPubkey,
              });
              if (entry.timelineKind === "comment") {
                return (
                  <li
                    className="rounded-xl border border-border bg-card/35 p-4"
                    data-testid="dispatch-comment"
                    key={entry.eventId}
                  >
                    <div className="flex items-center gap-2 text-sm">
                      <ProfileAvatar
                        profiles={profiles}
                        pubkey={entry.authorPubkey}
                      />
                      <strong>{label}</strong>
                      <time className="ml-auto text-xs text-muted-foreground">
                        {dateLabel(entry.createdAt)}
                      </time>
                    </div>
                    <Markdown className="mt-3 text-sm" content={entry.body} />
                  </li>
                );
              }
              const statusChange = entry.changes.find(
                (change) => change.field === "status",
              );
              return (
                <li
                  className="flex items-start gap-2 text-sm text-muted-foreground"
                  data-testid="dispatch-activity"
                  key={entry.eventId}
                >
                  <span className="font-medium text-foreground">{label}</span>
                  <span>
                    {entry.revision === 0 ? (
                      "created this issue"
                    ) : statusChange ? (
                      <>
                        moved{" "}
                        <strong className="text-foreground">
                          {titleCase(String(statusChange.before))} →{" "}
                          {titleCase(String(statusChange.after))}
                        </strong>
                      </>
                    ) : (
                      `revised ${entry.changes.map((change) => ({ body: "description", projectId: "project", project_id: "project", estimateMinutes: "estimate", estimate_minutes: "estimate", assignee: "owner", homeChannelId: "conversation", home_channel_id: "conversation", externalRefs: "links", external_refs: "links" })[change.field] ?? change.field).join(", ")}`
                    )}
                    {entry.reason ? <> — “{entry.reason}”</> : null}
                  </span>
                  <time className="ml-auto shrink-0 text-xs">
                    {dateLabel(entry.createdAt)}
                  </time>
                </li>
              );
            })}
          </ol>
        </section>

        <div className="mt-5 flex items-end gap-2 rounded-xl border border-border bg-card/30 p-3">
          <MessageSquare className="mb-2 h-4 w-4 shrink-0 text-muted-foreground" />
          <Textarea
            aria-label="Leave a comment"
            className="min-h-10 resize-none border-0 bg-transparent shadow-none focus-visible:ring-0"
            disabled={!canEdit || isMutating}
            onChange={(event) => setComment(event.target.value)}
            placeholder={
              canEdit ? "Leave a comment…" : "You are not authorized to comment"
            }
            value={comment}
          />
          <Button
            aria-label="Send comment"
            disabled={!canEdit || isMutating || !comment.trim()}
            onClick={() =>
              void onAddComment(comment.trim())
                .then(() => setComment(""))
                .catch(() => undefined)
            }
            size="icon"
            variant="ghost"
          >
            <Send className="h-4 w-4" />
          </Button>
        </div>
      </div>

      <aside className="overflow-y-auto border-l border-border px-6 py-6">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Properties
        </h2>
        <div className="mt-4">
          <Property label="Status">
            <select
              aria-label="Issue status"
              className="h-8 w-full rounded-md border border-border bg-muted/50 px-2 text-sm disabled:opacity-50"
              disabled={!canEdit || isMutating}
              onChange={(event) => {
                const status = event.target.value as DispatchStatus;
                if (!canTransition(role, issue.status, status)) return;
                void onMutate({
                  type: "status",
                  status,
                  reason: reason.trim() || null,
                })
                  .then(() => setReason(""))
                  .catch(() => undefined);
              }}
              value={issue.status}
            >
              {STATUSES.map((status) => (
                <option
                  disabled={!canTransition(role, issue.status, status)}
                  key={status}
                  value={status}
                >
                  {titleCase(status)}
                </option>
              ))}
            </select>
          </Property>
          <Property label="Reason">
            <input
              aria-label="Status change reason"
              className="h-8 w-full rounded-md border border-border bg-background px-2 text-sm disabled:opacity-50"
              data-testid="dispatch-status-reason"
              disabled={!canEdit || isMutating}
              onChange={(event) => setReason(event.target.value)}
              placeholder="Optional"
              value={reason}
            />
          </Property>
          <Property label="Priority">
            <select
              aria-label="Issue priority"
              className="h-8 w-full rounded-md border border-border bg-muted/50 px-2 text-sm disabled:opacity-50"
              disabled={!canEdit || isMutating}
              onChange={(event) =>
                void onMutate({
                  type: "priority",
                  priority: (event.target.value ||
                    null) as DispatchPriority | null,
                }).catch(() => undefined)
              }
              value={issue.priority ?? ""}
            >
              <option value="">No priority</option>
              {PRIORITIES.map((priority) => (
                <option key={priority}>{priority}</option>
              ))}
            </select>
          </Property>
          <Property label="Owner">
            <AssigneeControl
              canEdit={canEdit}
              currentPubkey={currentPubkey}
              isMutating={isMutating}
              onMutate={onMutate}
              profiles={profiles}
              pubkey={issue.assignee}
            />
          </Property>
          <Property label="Project">
            {sectionErrors.projects ? (
              <span className="text-xs text-muted-foreground">Unavailable</span>
            ) : project ? (
              <span className="inline-flex max-w-full items-center gap-1 rounded-md bg-muted px-2 py-1 text-xs">
                <FolderGit2 className="h-3 w-3" />
                <span className="truncate">{project.name}</span>
              </span>
            ) : (
              <em className="text-muted-foreground">None</em>
            )}
          </Property>
          <Property label="Labels">
            {issue.labels.length ? (
              <span className="flex flex-wrap gap-1">
                {issue.labels.map((label) => (
                  <Badge
                    className="normal-case tracking-normal"
                    key={label}
                    variant="secondary"
                  >
                    {label}
                  </Badge>
                ))}
              </span>
            ) : (
              <em className="text-muted-foreground">None</em>
            )}
          </Property>
          <Property label="Due">
            {issue.due ? (
              dateLabel(issue.due.value, true)
            ) : (
              <em className="text-muted-foreground">None</em>
            )}
          </Property>
          <Property label="Estimate">
            {issue.estimateMinutes === null ? (
              <em className="text-muted-foreground">None</em>
            ) : (
              <span>{issue.estimateMinutes} minutes</span>
            )}
          </Property>
          <Property label="Created">
            {dateLabel(issue.rootCreatedAt, true)}
          </Property>
          <Property label="Updated">
            {dateLabel(issue.tipCreatedAt, true)}
          </Property>
        </div>

        <section className="mt-5 border-t border-border pt-5">
          <h2 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            Provenance
          </h2>
          {imported ? (
            importedSourceUrl ? (
              <a
                className="mt-3 block rounded-lg border border-border p-3 text-xs hover:bg-muted/30"
                href={importedSourceUrl}
                rel="noreferrer"
              >
                <strong className="flex items-center gap-1">
                  <ExternalLink className="h-3.5 w-3.5" />
                  Imported from {titleCase(imported.system)}
                </strong>
                <span className="mt-1 block text-muted-foreground">
                  {imported.external_id} ·{" "}
                  {dateLabel(imported.imported_at, true)}
                </span>
              </a>
            ) : (
              <div className="mt-3 rounded-lg border border-border p-3 text-xs">
                <strong className="flex items-center gap-1">
                  <ExternalLink className="h-3.5 w-3.5" />
                  Imported from {titleCase(imported.system)}
                </strong>
                <span className="mt-1 block text-muted-foreground">
                  {imported.external_id} ·{" "}
                  {dateLabel(imported.imported_at, true)}
                </span>
              </div>
            )
          ) : (
            <p className="mt-3 text-sm italic text-muted-foreground">
              No import provenance
            </p>
          )}
        </section>
        <section className="mt-5 border-t border-border pt-5">
          <h2 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            Relations
          </h2>
          {relations.length ? (
            <div className="mt-2 space-y-1">
              {relations.map((relation) => {
                const targetIssue = issues.find(
                  (candidate) => candidate.issueId === relation.target,
                );
                const targetIdentifier =
                  targetIssue?.identifier ?? relation.target;
                return (
                  <button
                    className="flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted/40"
                    key={`${relation.label}:${relation.target}`}
                    onClick={() => onOpenIssue(relation.target)}
                    type="button"
                  >
                    <Link2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                    <span className="shrink-0">{relation.label}</span>
                    <span
                      className="ml-auto min-w-0 max-w-24 truncate font-mono text-xs text-muted-foreground"
                      title={targetIdentifier}
                    >
                      {targetIssue
                        ? issueAlias(targetIssue, issues)
                        : targetIdentifier}
                    </span>
                  </button>
                );
              })}
            </div>
          ) : (
            <p className="mt-3 text-sm italic text-muted-foreground">
              No relations
            </p>
          )}
        </section>

        {issue.externalRefs.length ? (
          <section className="mt-5 border-t border-border pt-5">
            <h2 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              Links and resources
            </h2>
            <div className="mt-2 space-y-1">
              {issue.externalRefs.map((reference) => (
                <div
                  className="rounded-md px-2 py-1.5 text-xs"
                  key={`${reference.type}:${reference.ref}`}
                >
                  <DispatchReferenceLink value={reference} />
                </div>
              ))}
            </div>
          </section>
        ) : null}
      </aside>
    </main>
  );
}
