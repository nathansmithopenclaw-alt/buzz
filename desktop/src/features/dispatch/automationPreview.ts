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
    if (!issue) return held("This issue is no longer available.");
    if (["done", "cancelled"].includes(issue.status))
      return held("This issue is already closed.");
    if (imported.has(id) && !activated.has(id))
      return held(
        "Open this imported issue and enable it for current work first.",
      );
    if (context.sectionErrors.tasks || context.sectionErrors.records)
      return held(
        "Refresh Dispatch so Buzz can check the latest work and settings.",
      );
    const route = controls.routing.find((route) =>
      issue.labels.includes(route.label),
    );
    if (route) {
      if (issue.status !== "todo")
        return held("Move this issue to Todo when it is ready to start.");
      if (blocked.has(id))
        return held("Waiting for work that blocks this issue.");
      if (
        context.linkedTasks.some(
          (task) =>
            task.issueId === id &&
            !["verified", "failed"].includes(task.status),
        )
      )
        return held(
          "This work is already with an agent or waiting for review.",
        );
      if (
        issue.externalRefs.some(
          (ref) =>
            ref.type === "task" &&
            !context.linkedTasks.some((task) => task.taskId === ref.ref),
        )
      )
        return held(
          "Buzz needs the linked task’s latest update before sending more work.",
        );
      if (!issue.assignee)
        return held("Choose an issue owner to review the agent’s result.");
      if (issue.assignee === route.executor)
        return held(
          "The issue owner must be someone other than the agent doing the work.",
        );
      const capacity = routingSuggestions(context, issue).find(
        (value) => value.executor === route.executor,
      );
      if (!capacity)
        return held("Add this agent under “Agents and work limits” first.");
      if (!capacity.eligible)
        return held(
          capacity.explanation
            .replaceAll("Executor is paused", "This agent is paused")
            .replaceAll("At capacity", "This agent already has enough work")
            .replaceAll(
              "No matching capability label",
              "This label is not listed in the agent’s work settings",
            ),
        );
      const inPreview = reserved.get(route.executor) ?? 0;
      if (capacity.used + inPreview >= capacity.limit)
        return held(
          "Other selected issues will use this agent’s available room.",
        );
      reserved.set(route.executor, inPreview + 1);
      return {
        id,
        title: issue.title,
        ready: true,
        reason: `Ready to send: matches “${route.label}”. The owner stays responsible for review.`,
      };
    }
    if (controls.notify_triage && issue.status === "triage" && !issue.assignee)
      return {
        id,
        title: issue.title,
        ready: true,
        reason: "Ready for a reminder if Buzz has not already sent it.",
      };
    if (controls.unassigned_after_secs && !issue.assignee)
      return {
        id,
        title: issue.title,
        ready: false,
        reason: "Buzz will check the waiting time before sending a reminder.",
      };
    return held("This issue does not match the jobs you chose.");
  });
}
