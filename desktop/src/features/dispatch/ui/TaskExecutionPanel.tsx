import { ExternalLink, ShieldCheck } from "lucide-react";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { TaskReviewDialog } from "./TaskReviewDialog";
import { Badge } from "@/shared/ui/badge";
import { taskMessageLink } from "../selectors.mjs";
import type {
  DispatchContext,
  DispatchIssue,
  DispatchLinkedTask,
} from "../types";

const labels = {
  assigned: "Assigned",
  picked_up: "Working",
  blocked: "Blocked",
  done: "Reported complete",
  failed: "Attempt failed",
  verified: "Independently verified",
};

function EvidenceReference({ value }: { value: string }) {
  let href: string | undefined;
  try {
    const url = new URL(value);
    if (["http:", "https:"].includes(url.protocol)) href = url.href;
  } catch {
    /* Plain artifact references stay readable. */
  }
  return href ? (
    <a
      className="break-all text-primary underline underline-offset-2"
      href={href}
      target="_blank"
      rel="noreferrer"
    >
      {value}
    </a>
  ) : (
    <span className="break-all">{value}</span>
  );
}

export function TaskExecutionPanel({
  issue,
  tasks,
  error,
  profiles,
  viewer,
  refresh,
}: {
  issue: DispatchIssue;
  tasks: readonly DispatchLinkedTask[];
  error: string | null;
  profiles?: UserProfileLookup;
  viewer?: string;
  refresh: () => Promise<DispatchContext>;
}) {
  const expected = issue.externalRefs.some((ref) =>
    ["task", "task_event"].includes(ref.type),
  );
  const name = (pubkey: string) =>
    pubkey === viewer ? "You" : resolveUserLabel({ profiles, pubkey });
  return (
    <section
      className="mt-5 border-t border-border pt-5"
      aria-label="Execution and evidence"
    >
      <h2 className="flex items-center gap-2 text-sm font-semibold">
        <ShieldCheck className="h-4 w-4" aria-hidden="true" />
        Execution and evidence
      </h2>
      {error ? (
        <p className="mt-3 rounded-md bg-warning-bg p-3 text-sm" role="status">
          Task history is unavailable. {error}
        </p>
      ) : null}
      {tasks.some((task) => task.admissionBasis === "legacy") ? (
        <p className="mt-2 text-xs text-muted-foreground">
          This conversation includes legacy task history evaluated with current
          permissions. Historical permission records need a maintainer's
          baseline review.
        </p>
      ) : null}
      {!tasks.length ? (
        <p className="mt-3 text-sm text-muted-foreground">
          {expected
            ? "This issue has a linked attempt, but its authorized history is unavailable. Refresh or open the source conversation to investigate."
            : "No linked task envelopes"}
        </p>
      ) : (
        <ol className="mt-3 space-y-3">
          {[...tasks]
            .sort((a, b) => b.createdAt - a.createdAt)
            .map((task) => (
              <li
                className="rounded-lg border border-border p-3"
                key={task.taskId}
                data-testid="dispatch-task-attempt"
              >
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant="outline">{labels[task.status]}</Badge>
                  <time
                    className="ml-auto text-xs text-muted-foreground"
                    dateTime={new Date(task.createdAt * 1000).toISOString()}
                  >
                    {new Date(task.createdAt * 1000).toLocaleString()}
                  </time>
                </div>
                <a
                  href={taskMessageLink(task)}
                  className="mt-2 flex items-center gap-2 font-medium text-sm"
                >
                  {task.title}
                  <ExternalLink
                    className="h-3.5 w-3.5 shrink-0"
                    aria-hidden="true"
                  />
                </a>
                <p className="mt-1 text-xs text-muted-foreground">
                  Executor: {name(task.assignee)}
                  {task.assigner ? (
                    <> · Requested by {name(task.assigner)}</>
                  ) : null}
                </p>
                {task.reviewer ? (
                  <p className="mt-1 text-xs text-muted-foreground">
                    Independent reviewer: {name(task.reviewer)}
                  </p>
                ) : null}
                {task.blockedOn ? (
                  <p className="mt-2 rounded bg-warning-bg p-2 text-sm">
                    {task.blockedOn}
                  </p>
                ) : null}
                {task.notes ? (
                  <p className="mt-2 whitespace-pre-wrap text-sm">
                    {task.notes}
                  </p>
                ) : null}
                {task.status === "done" ? (
                  <p className="mt-2 text-xs text-muted-foreground">
                    The executor reports completion. Independent acceptance is
                    still pending.
                  </p>
                ) : null}
                {task.status === "verified" && task.authorPubkey ? (
                  <p className="mt-2 text-xs">
                    Reviewed by {name(task.authorPubkey)}
                  </p>
                ) : null}
                {task.verification ? (
                  <div className="mt-3 border-t border-border pt-2 text-xs">
                    <p className="font-medium">Review method</p>
                    <p className="mt-1 whitespace-pre-wrap text-muted-foreground">
                      {task.verification.method}
                    </p>
                    {task.verification.evidence_ref ? (
                      <p className="mt-2">
                        <EvidenceReference
                          value={task.verification.evidence_ref}
                        />
                      </p>
                    ) : null}
                  </div>
                ) : null}
                {task.deliverables?.length ? (
                  <ul
                    className="mt-3 space-y-1 text-xs"
                    aria-label="Deliverables"
                  >
                    {task.deliverables.map((item) => (
                      <li key={`${item.type}:${item.ref}`}>
                        <span className="mr-1 text-muted-foreground">
                          {item.type}:
                        </span>
                        <EvidenceReference value={item.ref} />
                      </li>
                    ))}
                  </ul>
                ) : null}
                {task.history?.length ? (
                  <details className="mt-3 text-xs">
                    <summary className="cursor-pointer">
                      Attempt history · {task.history.length} updates
                    </summary>
                    <ol className="mt-2 space-y-3 border-l border-border pl-3">
                      {task.history.map((entry) => (
                        <li key={entry.eventId}>
                          <span className="font-medium">
                            {labels[entry.status]}
                          </span>{" "}
                          · {name(entry.authorPubkey)} ·{" "}
                          {new Date(entry.createdAt * 1000).toLocaleString()}
                          {entry.blockedOn ? (
                            <p className="mt-1 text-warning">
                              {entry.blockedOn}
                            </p>
                          ) : null}
                          {entry.notes ? (
                            <p className="mt-1 whitespace-pre-wrap">
                              {entry.notes}
                            </p>
                          ) : null}
                          {entry.evidence ? (
                            <EvidenceReference value={entry.evidence} />
                          ) : null}
                        </li>
                      ))}
                    </ol>
                  </details>
                ) : null}
                <TaskReviewDialog
                  task={task}
                  workspace={issue.streamId}
                  viewer={viewer}
                  refresh={refresh}
                />
                <details className="mt-2 text-xs text-muted-foreground">
                  <summary className="cursor-pointer">Record details</summary>
                  <dl className="mt-2 space-y-1 break-all">
                    <dt>Attempt</dt>
                    <dd>{task.taskId}</dd>
                    <dt>Latest signed update</dt>
                    <dd>{task.eventId}</dd>
                  </dl>
                </details>
              </li>
            ))}
        </ol>
      )}
    </section>
  );
}
