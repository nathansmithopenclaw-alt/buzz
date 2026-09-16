import { useCommunities } from "@/features/communities/useCommunities";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { AssigneePickerDialog } from "./AssigneePickerDialog";
import { pickAndUploadMedia } from "@/shared/api/tauri";
import * as React from "react";
import { Plus, X } from "lucide-react";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { Markdown } from "@/shared/ui/markdown";
import {
  dueDateInput,
  dueFromDateInput,
  issueAlias,
  type DispatchIssueEdit,
} from "../issueEditing";
import type { DispatchIssueMutation } from "../mutations";
import type {
  DispatchIssue,
  DispatchProject,
  DispatchRelation,
} from "../types";

const selectClass =
  "h-9 min-w-0 rounded-md border border-input bg-background px-2 text-sm";
const editableReferences = new Set([
  "url",
  "link",
  "pull_request",
  "pr",
  "commit",
  "doc",
  "file",
]);

function initialDraft(issue: DispatchIssue) {
  return {
    assignee: issue.assignee,
    homeChannelId: issue.homeChannelId ?? "",
    title: issue.title,
    body: issue.body,
    projectId: issue.projectId ?? "",
    labels: issue.labels.join(", "),
    due: dueDateInput(issue.due),
    estimate: issue.estimateMinutes?.toString() ?? "",
    relations: issue.relations,
    externalRefs: issue.externalRefs,
  };
}

export function IssueEditor({
  open,
  onOpenChange,
  issue,
  issues,
  projects,
  onSave,
  disabled,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  issue: DispatchIssue;
  issues: readonly DispatchIssue[];
  projects: readonly DispatchProject[];
  onSave: (mutation: DispatchIssueMutation) => Promise<DispatchIssue>;
  disabled: boolean;
}) {
  const { channels } = useChannelNavigation();
  const { activeCommunity } = useCommunities();
  const currentScope = React.useRef("");
  currentScope.current = `${activeCommunity?.relayUrl}:${issue.streamId}:${issue.issueId}`;
  const [ownerPicker, setOwnerPicker] = React.useState(false);
  const [base, setBase] = React.useState(issue);
  const [draft, setDraft] = React.useState(() => initialDraft(issue));
  const ownerProfiles = useUsersBatchQuery(
    draft.assignee ? [draft.assignee] : [],
  );
  const [error, setError] = React.useState<string | null>(null);
  const [uploading, setUploading] = React.useState(false);
  const [preview, setPreview] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [relationKind, setRelationKind] =
    React.useState<DispatchRelation["kind"]>("blocks");
  const [target, setTarget] = React.useState("");
  const [linkType, setLinkType] = React.useState("url");
  const [linkValue, setLinkValue] = React.useState("");
  const original = initialDraft(base);
  const dirty = JSON.stringify(draft) !== JSON.stringify(original);
  const changedElsewhere = issue.tipEvent !== base.tipEvent;
  React.useEffect(() => {
    if (!dirty && issue.tipEvent !== base.tipEvent) {
      setBase(issue);
      setDraft(initialDraft(issue));
    }
  }, [base.tipEvent, dirty, issue]);
  const change = <K extends keyof typeof draft>(
    key: K,
    value: (typeof draft)[K],
  ) => setDraft((current) => ({ ...current, [key]: value }));
  const fields = (): DispatchIssueEdit => {
    const result: DispatchIssueEdit = {};
    if (draft.assignee !== original.assignee) result.assignee = draft.assignee;
    if (draft.homeChannelId !== original.homeChannelId)
      result.homeChannelId = draft.homeChannelId || null;
    if (draft.title !== original.title) result.title = draft.title;
    if (draft.body !== original.body) result.body = draft.body;
    if (draft.projectId !== original.projectId)
      result.projectId = draft.projectId || null;
    if (draft.labels !== original.labels)
      result.labels = draft.labels.split(",");
    if (draft.due !== original.due) result.due = dueFromDateInput(draft.due);
    if (draft.estimate !== original.estimate)
      result.estimateMinutes =
        draft.estimate === "" ? null : Number(draft.estimate);
    if (JSON.stringify(draft.relations) !== JSON.stringify(original.relations))
      result.relations = draft.relations;
    if (
      JSON.stringify(draft.externalRefs) !==
      JSON.stringify(original.externalRefs)
    )
      result.externalRefs = draft.externalRefs;
    return result;
  };
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(null);
    try {
      const updated = await onSave({
        type: "edit",
        fields: fields(),
        expectedTip: base.tipEvent,
      });
      setBase(updated);
      setDraft(initialDraft(updated));
      onOpenChange(false);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  };
  const reviewLatest = () => {
    const latest = initialDraft(issue);
    for (const key of Object.keys(draft) as (keyof typeof draft)[]) {
      if (JSON.stringify(draft[key]) !== JSON.stringify(original[key]))
        Object.assign(latest, { [key]: draft[key] });
    }
    // Preserve newly linked execution records when reconciling an unrelated edit.
    latest.externalRefs = [
      ...latest.externalRefs,
      ...issue.externalRefs.filter(
        (ref) =>
          !editableReferences.has(ref.type) &&
          !latest.externalRefs.some(
            (value) => value.type === ref.type && value.ref === ref.ref,
          ),
      ),
    ];
    setBase(issue);
    setDraft(latest);
    setError(null);
  };
  const candidates = issues
    .filter(
      (candidate) =>
        candidate.issueId !== issue.issueId &&
        `${candidate.identifier} ${candidate.title}`
          .toLowerCase()
          .includes(query.toLowerCase()),
    )
    .slice(0, 40);
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!disabled) onOpenChange(next);
      }}
    >
      <DialogContent
        className="max-h-[90vh] overflow-y-auto sm:max-w-2xl"
        data-testid="dispatch-issue-editor"
      >
        <DialogHeader>
          <DialogTitle>Edit {issueAlias(issue, issues)}</DialogTitle>
          <DialogDescription>
            Update the details your teammates and agents use to understand this
            work.
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={submit}
          className="space-y-4"
          onKeyDown={(event) => {
            if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
              event.preventDefault();
              if (!disabled && !changedElsewhere)
                event.currentTarget.requestSubmit();
            }
          }}
        >
          <fieldset disabled={disabled || uploading} className="space-y-4">
            {changedElsewhere && dirty ? (
              <div
                className="rounded-lg border border-warning/40 bg-warning-bg p-3 text-sm"
                role="status"
              >
                <p>
                  This issue changed while you were editing. Your draft is
                  preserved.
                </p>
                <details className="mt-2">
                  <summary className="cursor-pointer">
                    Compare the latest version
                  </summary>
                  <p className="mt-2 font-medium">{issue.title}</p>
                  <Markdown className="mt-2 text-sm" content={issue.body} />
                </details>
                <Button
                  type="button"
                  variant="outline"
                  className="mt-2"
                  onClick={reviewLatest}
                >
                  Keep my draft on the latest version
                </Button>
              </div>
            ) : null}
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="grid gap-1.5 text-sm">
                <span>Responsible owner</span>
                <Button
                  variant="outline"
                  className="justify-start"
                  onClick={() => setOwnerPicker(true)}
                >
                  {draft.assignee
                    ? resolveUserLabel({
                        pubkey: draft.assignee,
                        profiles: ownerProfiles.data?.profiles,
                      })
                    : "Choose owner"}
                </Button>
              </div>
              <label className="grid gap-1.5 text-sm">
                Conversation
                <select
                  className={selectClass}
                  value={draft.homeChannelId}
                  onChange={(event) =>
                    change("homeChannelId", event.target.value)
                  }
                >
                  <option value="">No home conversation</option>
                  {channels.map((channel) => (
                    <option key={channel.id} value={channel.id}>
                      #{channel.name}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <AssigneePickerDialog
              open={ownerPicker}
              onOpenChange={setOwnerPicker}
              assignee={draft.assignee}
              disabled={disabled}
              profiles={ownerProfiles.data?.profiles}
              onAssign={(pubkey) => {
                change("assignee", pubkey);
                setOwnerPicker(false);
              }}
            />
            <label
              className="grid gap-1.5 text-sm font-medium"
              htmlFor="dispatch-edit-title"
            >
              Title
              <Input
                id="dispatch-edit-title"
                autoFocus
                required
                maxLength={256}
                value={draft.title}
                onChange={(event) => change("title", event.target.value)}
                data-testid="dispatch-edit-title"
              />
            </label>
            <div>
              <div className="mb-1.5 flex items-center justify-between">
                <label
                  className="text-sm font-medium"
                  htmlFor="dispatch-edit-description"
                >
                  Description
                </label>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  aria-pressed={preview}
                  onClick={() => setPreview(!preview)}
                >
                  {preview ? "Write" : "Preview"}
                </Button>
              </div>
              {preview ? (
                <div className="min-h-32 rounded-md border p-3">
                  <Markdown content={draft.body || "No description yet."} />
                </div>
              ) : (
                <Textarea
                  id="dispatch-edit-description"
                  rows={7}
                  value={draft.body}
                  onChange={(event) => change("body", event.target.value)}
                  placeholder="Outcome, context, acceptance criteria… Markdown is supported."
                />
              )}
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <label className="grid gap-1.5 text-sm">
                Project
                <select
                  className={selectClass}
                  value={draft.projectId}
                  onChange={(event) => change("projectId", event.target.value)}
                >
                  <option value="">No project</option>
                  {projects.map((project) => (
                    <option key={project.projectId} value={project.projectId}>
                      {project.name}
                    </option>
                  ))}
                </select>
              </label>
              <label
                className="grid gap-1.5 text-sm"
                htmlFor="dispatch-edit-labels"
              >
                Labels
                <Input
                  id="dispatch-edit-labels"
                  value={draft.labels}
                  onChange={(event) => change("labels", event.target.value)}
                  placeholder="design, customer-feedback"
                />
              </label>
              <label
                className="grid gap-1.5 text-sm"
                htmlFor="dispatch-edit-due"
              >
                Due date
                <Input
                  id="dispatch-edit-due"
                  type="date"
                  value={draft.due}
                  onChange={(event) => change("due", event.target.value)}
                />
              </label>
              <label
                className="grid gap-1.5 text-sm"
                htmlFor="dispatch-edit-estimate"
              >
                Estimate in minutes <span className="sr-only">Optional</span>
                <Input
                  id="dispatch-edit-estimate"
                  type="number"
                  min={0}
                  step={1}
                  value={draft.estimate}
                  onChange={(event) => change("estimate", event.target.value)}
                  placeholder="Optional"
                />
              </label>
            </div>
            <fieldset className="rounded-lg border border-border p-3">
              <legend className="px-1 text-sm font-medium">
                Relationships
              </legend>
              <ul className="space-y-2">
                {draft.relations.map((relation) => (
                  <li
                    className="flex items-center gap-2 text-sm"
                    key={`${relation.kind}:${relation.targetIssueId}`}
                  >
                    <span>
                      {relation.kind.replaceAll("_", " ")} ·{" "}
                      {issues.find(
                        (value) => value.issueId === relation.targetIssueId,
                      )?.title ?? "Unavailable issue"}
                    </span>
                    <Button
                      className="ml-auto shrink-0"
                      size="icon-xs"
                      variant="ghost"
                      type="button"
                      aria-label="Remove relationship"
                      onClick={() =>
                        change(
                          "relations",
                          draft.relations.filter((value) => value !== relation),
                        )
                      }
                    >
                      <X />
                    </Button>
                  </li>
                ))}
              </ul>
              <Input
                className="mt-2"
                aria-label="Find an issue to relate"
                placeholder="Find an issue…"
                value={query}
                onChange={(event) => {
                  setQuery(event.target.value);
                  setTarget("");
                }}
              />
              <div className="mt-2 flex gap-2">
                <select
                  className={selectClass}
                  aria-label="Relationship"
                  value={relationKind}
                  onChange={(event) =>
                    setRelationKind(
                      event.target.value as DispatchRelation["kind"],
                    )
                  }
                >
                  <option value="blocks">Blocks</option>
                  <option value="parent">Parent of</option>
                  <option value="duplicate_of">Duplicate of</option>
                </select>
                <select
                  className={`${selectClass} flex-1`}
                  aria-label="Related issue"
                  value={target}
                  onChange={(event) => setTarget(event.target.value)}
                >
                  <option value="">Choose issue</option>
                  {candidates.map((candidate) => (
                    <option key={candidate.issueId} value={candidate.issueId}>
                      {issueAlias(candidate, issues)} · {candidate.title}
                    </option>
                  ))}
                </select>
                <Button
                  size="icon"
                  type="button"
                  aria-label="Add relationship"
                  disabled={!target}
                  onClick={() => {
                    const candidate = issues.find(
                      (value) => value.issueId === target,
                    );
                    if (
                      candidate &&
                      !draft.relations.some(
                        (value) =>
                          value.kind === relationKind &&
                          value.targetIssueId === target,
                      )
                    )
                      change("relations", [
                        ...draft.relations,
                        {
                          kind: relationKind,
                          targetIssueId: target,
                          targetRootEvent: candidate.rootEvent,
                        },
                      ]);
                    setTarget("");
                  }}
                >
                  <Plus />
                </Button>
              </div>
            </fieldset>
            <div className="flex items-center gap-3">
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={uploading || disabled}
                onClick={() => {
                  setUploading(true);
                  setError(null);
                  const scope = currentScope.current;
                  void pickAndUploadMedia()
                    .then((files) => {
                      if (scope !== currentScope.current)
                        throw new Error(
                          "Workspace changed while choosing the attachment. Return to the original issue to attach it.",
                        );
                      setDraft((current) => ({
                        ...current,
                        externalRefs: [
                          ...current.externalRefs,
                          ...files.map((file) => ({
                            type: "file",
                            ref: file.url,
                          })),
                        ],
                      }));
                    })
                    .catch((error) => setError(String(error)))
                    .finally(() => setUploading(false));
                }}
              >
                {uploading ? "Uploading…" : "Attach files"}
              </Button>
              <span className="text-xs text-muted-foreground">
                Attachments will be visible with this issue after you save.
              </span>
            </div>
            <fieldset className="rounded-lg border border-border p-3">
              <legend className="px-1 text-sm font-medium">
                Links and artifacts
              </legend>
              <ul className="space-y-2">
                {draft.externalRefs
                  .filter((ref) => editableReferences.has(ref.type))
                  .map((ref) => (
                    <li
                      className="flex items-center gap-2 text-sm"
                      key={`${ref.type}:${ref.ref}`}
                    >
                      <span className="min-w-0 break-all">
                        {ref.type}: {ref.ref}
                      </span>
                      <Button
                        type="button"
                        size="icon-xs"
                        variant="ghost"
                        aria-label="Remove link"
                        className="ml-auto shrink-0"
                        onClick={() =>
                          change(
                            "externalRefs",
                            draft.externalRefs.filter((value) => value !== ref),
                          )
                        }
                      >
                        <X />
                      </Button>
                    </li>
                  ))}
              </ul>
              <div className="mt-2 flex gap-2">
                <select
                  className={selectClass}
                  aria-label="Link type"
                  value={linkType}
                  onChange={(event) => setLinkType(event.target.value)}
                >
                  <option value="url">Link</option>
                  <option value="pull_request">Pull request</option>
                  <option value="commit">Commit</option>
                  <option value="doc">Document</option>
                  <option value="file">File</option>
                </select>
                <Input
                  aria-label="Link or artifact reference"
                  placeholder="URL or artifact reference"
                  value={linkValue}
                  onChange={(event) => setLinkValue(event.target.value)}
                />
                <Button
                  type="button"
                  size="icon"
                  aria-label="Add link"
                  disabled={!linkValue.trim()}
                  onClick={() => {
                    const ref = { type: linkType, ref: linkValue.trim() };
                    if (
                      !draft.externalRefs.some(
                        (value) =>
                          value.type === ref.type && value.ref === ref.ref,
                      )
                    )
                      change("externalRefs", [...draft.externalRefs, ref]);
                    setLinkValue("");
                  }}
                >
                  <Plus />
                </Button>
              </div>
            </fieldset>
            {error ? (
              <p
                className="rounded-md border border-destructive/30 p-3 text-sm text-destructive"
                role="alert"
              >
                {error}
              </p>
            ) : null}
            <DialogFooter>
              <Button
                type="button"
                variant="ghost"
                disabled={disabled}
                onClick={() => onOpenChange(false)}
              >
                Close{dirty ? " and keep draft" : ""}
              </Button>
              <Button
                type="submit"
                disabled={disabled || !dirty || changedElsewhere}
                data-testid="dispatch-save-issue"
              >
                {disabled ? "Saving…" : "Save changes"}
              </Button>
            </DialogFooter>
          </fieldset>
        </form>
      </DialogContent>
    </Dialog>
  );
}
