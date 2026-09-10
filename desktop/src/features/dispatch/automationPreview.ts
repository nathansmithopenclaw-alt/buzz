import { blockedIssueIds } from "./viewModel";
import { activeRecords, type AutomationControls } from "./recordTypes";
import { routingSuggestions } from "./attention";
import type { DispatchContext } from "./types";
/** Advisory preview. The resident service independently rechecks authority and budgets. */
export function automationPreview(
  context: DispatchContext,
  controls: AutomationControls,
) {
  const blocked = blockedIssueIds(context.issues);
  const activated = new Set(
    activeRecords(context.records, "activation")
      .filter((value) => value.record.payload.active)
      .map((value) => value.record.payload.issue_id),
  );
  const imported = new Set(
    context.activity
      .filter((value) => value.import)
      .map((value) => value.issueId),
  );
  const reserved = new Map<string, number>();
  return controls.issue_ids.map((id) => {
    const issue = context.issues.find((value) => value.issueId === id);
    const held = (reason: string) => ({
      id,
      title: issue?.title ?? "Unavailable issue",
      ready: false,
      reason,
    });
    if (!issue) return held("Issue is unavailable");
    if (["done", "cancelled"].includes(issue.status))
      return held("Issue is closed");
    if (imported.has(id) && !activated.has(id))
      return held("Imported work is parked");
    if (context.sectionErrors.tasks || context.sectionErrors.records)
      return held("Refresh incomplete task or planning history");
    const route = controls.routing.find((route) =>
      issue.labels.includes(route.label),
    );
    if (route) {
      if (issue.status !== "todo") return held("Accept into Todo first");
      if (blocked.has(id)) return held("Unfinished dependency");
      if (
        context.linkedTasks.some(
          (task) =>
            task.issueId === id &&
            !["verified", "failed"].includes(task.status),
        )
      )
        return held("An attempt is active or awaiting review");
      if (
        issue.externalRefs.some(
          (ref) =>
            ref.type === "task" &&
            !context.linkedTasks.some((task) => task.taskId === ref.ref),
        )
      )
        return held("Linked task history is unavailable");
      if (!issue.assignee)
        return held("Choose an accountable owner and reviewer");
      if (issue.assignee === route.executor)
        return held("Choose an independent reviewer");
      const capacity = routingSuggestions(context, issue).find(
        (value) => value.executor === route.executor,
      );
      if (!capacity) return held("Configure executor capacity");
      if (!capacity.eligible) return held(capacity.explanation);
      const inPreview = reserved.get(route.executor) ?? 0;
      if (capacity.used + inPreview >= capacity.limit)
        return held("Earlier candidates reserve the remaining capacity");
      reserved.set(route.executor, inPreview + 1);
      return {
        id,
        title: issue.title,
        ready: true,
        reason: `Route by ${route.label}; retain current owner`,
      };
    }
    if (controls.notify_triage && issue.status === "triage" && !issue.assignee)
      return {
        id,
        title: issue.title,
        ready: true,
        reason: "One triage notice, if not previously delivered",
      };
    if (controls.unassigned_after_secs && !issue.assignee)
      return {
        id,
        title: issue.title,
        ready: false,
        reason: "Notice becomes eligible after the configured waiting period",
      };
    return held("No enabled rule applies");
  });
}
