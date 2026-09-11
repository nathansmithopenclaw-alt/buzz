import { loadAdmissionSources } from "./admissionHistory";
import {
  foldTrackerContext,
  foldTrackerContextIncremental,
  resumeTrackerContext,
  trackerFoldCursor,
  readTrackerContextCache,
  readTrackerMembership,
} from "@/shared/api/tracker";
import { isDispatchHistoryGap, type DispatchHistoryClient } from "./history";
import { dispatchHistoryClient } from "./relayHistory";
import { loadLinkedTaskHistory } from "./linkedTaskHistory";
import type { DispatchContext } from "./types";
const TRACKER_HISTORY_KINDS = [40020, 40021];
const TRACKER_COMMENT_KIND = 40022;
const PROJECT_KIND = 30621;
const TASK_KIND = 9;
const TASK_DISCOVERY_TAG = "buzz-task";

type FoldedContext = Omit<DispatchContext, "sectionErrors">;
const nativeReads = {
  membership: readTrackerMembership,
  cursor: trackerFoldCursor,
  preview: readTrackerContextCache<DispatchContext>,
  fold: foldTrackerContext<FoldedContext>,
  resume: resumeTrackerContext<FoldedContext>,
  incremental: foldTrackerContextIncremental<FoldedContext>,
};
export type DispatchContextReadOptions = {
  signal?: AbortSignal;
  includePreview?: boolean;
  history?: DispatchHistoryClient;
  native?: typeof nativeReads;
};

export async function loadDispatchContext(
  workspaceId: string,
  managedSigner: string,
  onRetryingChange: (value: boolean) => void,
  onPreview: (value: DispatchContext) => void = () => {},
  options: DispatchContextReadOptions = {},
): Promise<DispatchContext> {
  const source =
    options.history ?? dispatchHistoryClient(onRetryingChange, options.signal);
  const native = options.native ?? nativeReads;
  const checkActive = () => options.signal?.throwIfAborted();
  const run = async <T>(operation: () => Promise<T>): Promise<T> => {
    checkActive();
    const result = await operation();
    checkActive();
    return result;
  };
  const history: DispatchHistoryClient = {
    fetch: (filter) => run(() => source.fetch(filter)),
    fetchMulti: (filters) => run(() => source.fetchMulti(filters)),
    fetchExhaustively: (filter) => run(() => source.fetchExhaustively(filter)),
  };
  const snapshotAt = Math.floor(Date.now() / 1000);
  // Membership and local cache loading do not depend on the renderer's
  // startup subscription replay. Current membership still gates the preview.
  const [membershipEvent, cursor] = await Promise.all([
    run(() => native.membership({ workspaceId, managedSigner })),
    native.cursor({ workspaceId, managedSigner }).catch(() => null),
  ]);
  if (!membershipEvent)
    throw new Error("Tracker workspace membership snapshot is unavailable.");
  if (options.includePreview !== false) {
    // Older caches can still supply a read-only preview while their first
    // complete records sync starts from the beginning.
    const preview = await native
      .preview({
        workspaceId,
        managedSigner,
        membershipEvent,
      })
      .catch(() => null);
    checkActive();
    if (preview) onPreview(preview);
  }
  const fetchStreams = (since: number | null) =>
    Promise.allSettled([
      history.fetchExhaustively({
        kinds: TRACKER_HISTORY_KINDS,
        "#h": [workspaceId],
        ...(since === null ? {} : { since }),
      }),
      history.fetchExhaustively({
        kinds: [TRACKER_COMMENT_KIND],
        "#h": [workspaceId],
        ...(since === null ? {} : { since }),
      }),
      history.fetchExhaustively({
        kinds: [PROJECT_KIND],
        "#buzz-workspace": [workspaceId],
        ...(since === null ? {} : { since }),
      }),
      history.fetchExhaustively({
        kinds: [40026, 40023, 40024, 40027],
        "#h": [workspaceId],
        ...(since === null ? {} : { since }),
      }),
      history.fetchExhaustively({
        kinds: [TASK_KIND],
        "#t": [TASK_DISCOVERY_TAG],
        "#h": [workspaceId],
        ...(since === null ? {} : { since }),
      }),
    ]);
  let warm = cursor !== null;
  let [trackerHistory, comments, projects, records, tasks] =
    await fetchStreams(cursor);
  if (
    warm &&
    [trackerHistory, comments, projects, records, tasks].some(
      (result) =>
        result.status === "rejected" && isDispatchHistoryGap(result.reason),
    )
  ) {
    warm = false;
    [trackerHistory, comments, projects, records, tasks] =
      await fetchStreams(null);
  }
  if (trackerHistory.status === "rejected") throw trackerHistory.reason;
  const events = [
    ...trackerHistory.value,
    ...(records.status === "fulfilled" ? records.value : []),
    ...(comments.status === "fulfilled" ? comments.value : []),
    ...(projects.status === "fulfilled" ? projects.value : []),
  ];
  if (records.status === "fulfilled") {
    events.push(
      ...(await loadAdmissionSources(
        history,
        records.value.filter(
          (event) => !warm || event.created_at >= (cursor ?? 0),
        ),
        workspaceId,
        new Set(events.map((event) => event.id)),
      )),
    );
  }
  const taskEvents = [
    ...(tasks.status === "fulfilled" ? tasks.value : []),
    ...events.filter((event) => event.kind === TASK_KIND),
  ];
  checkActive();
  let folded = warm
    ? await native.resume({
        events,
        taskEvents,
        membershipEvent,
        workspaceId,
        managedSigner,
        snapshotAt,
        advanceCursor: [records, comments, projects, tasks].every(
          (result) => result.status === "fulfilled",
        ),
      })
    : await native.fold({
        events,
        taskEvents,
        membershipEvent,
        workspaceId,
        managedSigner,
        snapshotAt,
        advanceCursor: [records, comments, projects, tasks].every(
          (result) => result.status === "fulfilled",
        ),
      });
  let relatedError: string | null = null;
  let linkedTaskScope: DispatchContext["linkedTaskScope"];
  try {
    const related = await loadLinkedTaskHistory(
      history,
      folded.issues,
      workspaceId,
      managedSigner,
    );
    linkedTaskScope = {
      channelIds: related.channelIds,
      taskIds: related.taskIds,
    };
    if (related.events.length || related.taskEvents.length) {
      folded = await native.incremental({
        events: related.events,
        taskEvents: related.taskEvents,
        workspaceId,
        managedSigner,
        snapshotAt,
      });
    }
  } catch (error) {
    relatedError = error instanceof Error ? error.message : String(error);
  }
  checkActive();
  const errorMessage = (result: PromiseSettledResult<unknown>) =>
    result.status === "rejected"
      ? result.reason instanceof Error
        ? result.reason.message
        : String(result.reason)
      : null;
  return {
    ...folded,
    linkedTaskScope,
    sectionErrors: {
      records: errorMessage(records),
      comments: errorMessage(comments),
      projects: errorMessage(projects),
      tasks: errorMessage(tasks) ?? relatedError,
    },
  };
}
