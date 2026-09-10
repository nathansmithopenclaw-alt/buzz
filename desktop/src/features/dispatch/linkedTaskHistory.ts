import type { RelayEvent } from "@/shared/api/types";
import type { DispatchHistoryClient } from "./history";
import type { DispatchIssue } from "./types";

const CHUNK = 50;
const unique = (values: string[]) => [...new Set(values)].sort();
const tag = (event: RelayEvent, key: string) =>
  event.tags.find((tag) => tag[0] === key)?.[1];

/** Explicit issue references define discovery scope; the relay enforces channel access. */
export function linkedTaskReferences(issues: DispatchIssue[]) {
  return {
    taskIds: unique(
      issues.flatMap((issue) =>
        issue.externalRefs
          .filter((ref) => ref.type === "task")
          .map((ref) => ref.ref),
      ),
    ),
    eventIds: unique(
      issues.flatMap((issue) =>
        issue.externalRefs
          .filter((ref) => ref.type === "task_event")
          .map((ref) => ref.ref),
      ),
    ),
  };
}

/** Fetch complete, explicitly linked attempt history, including its original assignment. */
export async function loadLinkedTaskHistory(
  history: DispatchHistoryClient,
  issues: DispatchIssue[],
  workspaceId: string,
  viewer: string,
): Promise<{
  events: RelayEvent[];
  taskEvents: RelayEvent[];
  channelIds: string[];
  taskIds: string[];
}> {
  const refs = linkedTaskReferences(issues);
  if (!refs.taskIds.length && !refs.eventIds.length)
    return { events: [], taskEvents: [], channelIds: [], taskIds: [] };
  const anchors: RelayEvent[] = [];
  for (let i = 0; i < refs.eventIds.length; i += CHUNK) {
    anchors.push(
      ...(await history.fetch({
        kinds: [9],
        ids: refs.eventIds.slice(i, i + CHUNK),
        limit: CHUNK,
      })),
    );
  }
  // A task UUID is a complete explicit link even when no message permalink or
  // home conversation was saved. Discover only those referenced task IDs;
  // the relay filters authorized channels, and the roster check below still
  // governs what enters the native projection.
  const anchoredIds = new Set(anchors.map((event) => tag(event, "task")));
  const unresolvedIds = refs.taskIds.filter((id) => !anchoredIds.has(id));
  for (let i = 0; i < unresolvedIds.length; i += CHUNK) {
    anchors.push(
      ...(await history.fetchExhaustively({
        kinds: [9],
        "#task": unresolvedIds.slice(i, i + CHUNK),
        "#t": ["buzz-task"],
      })),
    );
  }
  const channels = unique([
    ...issues
      .filter((issue) =>
        issue.externalRefs.some(
          (ref) => ref.type === "task" || ref.type === "task_event",
        ),
      )
      .flatMap((issue) => (issue.homeChannelId ? [issue.homeChannelId] : [])),
    ...anchors.flatMap((event) =>
      tag(event, "h") ? [tag(event, "h") as string] : [],
    ),
  ]).filter((channel) => channel !== workspaceId);
  const memberships: RelayEvent[] = [];
  for (let i = 0; i < channels.length; i += CHUNK) {
    memberships.push(
      ...(await history.fetchExhaustively({
        kinds: [39002],
        "#d": channels.slice(i, i + CHUNK),
      })),
    );
  }
  const allowed = channels.filter((channel) => {
    const latest = memberships
      .filter((event) => tag(event, "d") === channel)
      .sort(
        (a, b) => b.created_at - a.created_at || b.id.localeCompare(a.id),
      )[0];
    return latest?.tags.some(
      (entry) => entry[0] === "p" && entry[1] === viewer,
    );
  });
  const taskIds = unique([
    ...refs.taskIds,
    ...anchors.flatMap((event) =>
      tag(event, "task") ? [tag(event, "task") as string] : [],
    ),
  ]);
  const taskEvents = new Map(
    anchors
      .filter(
        (event) =>
          allowed.includes(tag(event, "h") ?? "") ||
          tag(event, "h") === workspaceId,
      )
      .map((event) => [event.id, event]),
  );
  for (const channel of allowed) {
    for (let i = 0; i < taskIds.length; i += CHUNK) {
      const events = await history.fetchExhaustively({
        kinds: [9],
        "#h": [channel],
        "#task": taskIds.slice(i, i + CHUNK),
        "#t": ["buzz-task"],
      });
      for (const event of events) taskEvents.set(event.id, event);
    }
  }
  const receipts: RelayEvent[] = [];
  for (const channel of allowed) {
    const ids = [...taskEvents.values()]
      .filter((event) => tag(event, "h") === channel)
      .map((event) => event.id);
    for (let i = 0; i < ids.length; i += CHUNK) {
      receipts.push(
        ...(await history.fetchExhaustively({
          kinds: [40027],
          "#h": [channel],
          "#e": ids.slice(i, i + CHUNK),
        })),
      );
    }
  }
  return {
    events: [...memberships, ...receipts],
    taskEvents: [...taskEvents.values()],
    channelIds: allowed,
    taskIds,
  };
}
