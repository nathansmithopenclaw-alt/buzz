import * as React from "react";
import { loadAdmissionSources } from "./admissionHistory";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { useIdentityQuery } from "@/shared/api/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { MAX_FILTERS_PER_REQ } from "@/shared/api/relayClientShared";

import { foldTrackerContextIncremental } from "@/shared/api/tracker";
import type { RelayEvent } from "@/shared/api/types";
import type { DispatchHistoryClient } from "./history";
import { loadDispatchContext } from "./contextLoader";
import { getStorageItem } from "@/shared/lib/safeStorage";
import { dispatchHistoryClient } from "./relayHistory";
import { createDispatchLiveMerge, type DispatchLiveMerge } from "./liveMerge";
import {
  linkedTaskReferences,
  loadLinkedTaskHistory,
} from "./linkedTaskHistory";
import type { DispatchContext, DispatchWorkspace } from "./types";

// Fold-consumed issue/revision/comment kinds. Kind 40025 is M20 project-activity
// history (`buzz track project get --history`); Dispatch has no consumer, so the
// live sub and populated-workspace probe must not fetch it.
export const TRACKER_KINDS = [40020, 40021, 40022];
export const TRACKER_HISTORY_KINDS = [40020, 40021];
const PROJECT_KIND = 30621;
const TASK_KIND = 9;
const TASK_DISCOVERY_TAG = "buzz-task";
const LIVE_REFRESH_DEBOUNCE_MS = 250;

function asError(value: unknown): Error {
  return value instanceof Error
    ? value
    : new Error(`Dispatch live subscription failed: ${String(value)}`);
}

function tag(event: RelayEvent, name: string): string | null {
  return event.tags.find((value) => value[0] === name)?.[1] ?? null;
}

function workspaceName(event: RelayEvent | undefined, id: string): string {
  if (!event) return id;
  const tagged = tag(event, "name");
  if (tagged) return tagged;
  try {
    const parsed = JSON.parse(event.content) as { name?: unknown };
    if (typeof parsed.name === "string" && parsed.name.trim())
      return parsed.name;
  } catch {
    // NIP-29 metadata may use tags without JSON content.
  }
  return id;
}

/**
 * Ask which of `ids` hold at least one tracker event, in
 * `ceil(ids.length / MAX_FILTERS_PER_REQ)` requests rather than one per id.
 *
 * Each workspace gets its own `limit: 1` filter, batched into multi-filter
 * REQs. The relay evaluates filters independently, so one busy workspace
 * cannot consume another's limit — the reason this is not a single combined
 * `#h` filter, which would be faster still but would silently misreport
 * workspaces as empty once any one of them saturates the shared limit.
 *
 * The relay does not say which filter matched an event, so attribution comes
 * from the event's own `h` tag. An event carrying several `h` tags marks every
 * workspace it belongs to, which is exactly the per-filter answer.
 */
export async function probePopulatedWorkspaces(
  history: Pick<DispatchHistoryClient, "fetchMulti">,
  ids: string[],
): Promise<Set<string>> {
  const wanted = new Set(ids);
  const populated = new Set<string>();
  for (let index = 0; index < ids.length; index += MAX_FILTERS_PER_REQ) {
    const batch = ids.slice(index, index + MAX_FILTERS_PER_REQ);
    const events = await history.fetchMulti(
      batch.map((id) => ({ kinds: TRACKER_KINDS, "#h": [id], limit: 1 })),
    );
    for (const event of events) {
      for (const value of event.tags) {
        if (value[0] === "h" && wanted.has(value[1])) populated.add(value[1]);
      }
    }
  }
  return populated;
}

async function loadWorkspaces(pubkey: string): Promise<DispatchWorkspace[]> {
  const history = dispatchHistoryClient();
  const memberships = await history.fetchExhaustively({
    kinds: [39002],
    "#p": [pubkey],
  });
  const ids = [
    ...new Set(memberships.map((event) => tag(event, "d")).filter(Boolean)),
  ] as string[];
  if (ids.length === 0) return [];
  const metadata = await history.fetchExhaustively({
    kinds: [39000],
    "#d": ids,
  });
  const populated = await probePopulatedWorkspaces(history, ids);
  return ids.map((id) => {
    const meta = metadata
      .filter((event) => tag(event, "d") === id)
      .sort((a, b) => b.created_at - a.created_at)[0];
    return {
      id,
      name: workspaceName(meta, id),
      // Probed with `limit: 1` per workspace, so this has always been a 0/1
      // "is it populated" flag rather than a true count; every consumer reads
      // it as a boolean.
      trackerEventCount: populated.has(id) ? 1 : 0,
    };
  });
}

/**
 * Workspace membership changes rarely, and the list is re-read every time
 * Dispatch mounts. Without this the default `staleTime: 0` means every remount
 * pays full discovery again — not just once per launch.
 *
 * Matched to the client's `gcTime` rather than set higher. Nothing invalidates
 * this key, so `staleTime` is the *only* thing bounding how long a
 * newly-joined workspace stays invisible; going past `gcTime` would buy no
 * extra cache hits in the unmount-and-return case (the entry is collected
 * anyway) while widening that window. The Dispatch refresh control forces a
 * refetch inside it.
 */
const WORKSPACES_STALE_TIME_MS = 5 * 60 * 1_000;

/**
 * Shared so the boot prefetch and the screen's own read resolve to the same
 * cache entry. A prefetch under a different key or `staleTime` warms nothing.
 */
function dispatchWorkspacesQuery(pubkey: string | undefined) {
  return {
    queryKey: ["dispatch", "workspaces", pubkey] as const,
    queryFn: () => loadWorkspaces(pubkey ?? ""),
    staleTime: WORKSPACES_STALE_TIME_MS,
  };
}

/**
 * Warm the workspace list before the user first opens Dispatch, so discovery
 * is not on the critical path of that click.
 *
 * Callers pass a pubkey that is already deferred past first paint: boot is
 * when channel re-subscription is spending the relay's per-pubkey frame
 * budget, and this is speculative work for a screen the user may never visit.
 */
function dispatchContextQuery(
  workspaceId: string,
  managedSigner: string,
  queryClient: ReturnType<typeof useQueryClient>,
  onRetryingChange: (value: boolean) => void = () => {},
) {
  const queryKey = ["dispatch", "context", workspaceId, managedSigner] as const;
  return {
    queryKey,
    queryFn: ({ signal }: { signal: AbortSignal }) =>
      loadDispatchContext(
        workspaceId,
        managedSigner,
        onRetryingChange,
        (preview) => {
          queryClient.setQueryData([...queryKey, "preview"], preview);
        },
        { signal, includePreview: !queryClient.getQueryData(queryKey) },
      ),
    // Returning to Dispatch paints cached data immediately and revalidates.
    gcTime: 30 * 60 * 1000,
  };
}

function prefetchDispatchContext(
  queryClient: ReturnType<typeof useQueryClient>,
  workspace: string,
  pubkey: string,
) {
  const options = dispatchContextQuery(workspace, pubkey, queryClient);
  if (!queryClient.getQueryState(options.queryKey))
    void queryClient.prefetchQuery(options);
}

export function useDispatchWorkspacePrefetch(pubkey: string | undefined) {
  const queryClient = useQueryClient();
  React.useEffect(() => {
    if (!pubkey) return;
    let cancelled = false;
    const preferred = getStorageItem("buzz-dispatch-last-workspace-v1");
    // Route data and its JS chunk warm after first paint, using the same query
    // as the screen. Relay membership still gates every cached preview.
    void import("./ui/DispatchScreen").catch(() => undefined);
    if (preferred) prefetchDispatchContext(queryClient, preferred, pubkey);
    void queryClient
      .fetchQuery(dispatchWorkspacesQuery(pubkey))
      .then((workspaces) => {
        if (
          cancelled ||
          workspaces.some((workspace) => workspace.id === preferred)
        )
          return;
        const populated = workspaces.filter(
          (workspace) => workspace.trackerEventCount > 0,
        );
        if (populated.length === 1)
          prefetchDispatchContext(queryClient, populated[0].id, pubkey);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      void queryClient.cancelQueries({ queryKey: ["dispatch"] });
    };
  }, [pubkey, queryClient]);
}

export function useDispatchWorkspaces() {
  const identity = useIdentityQuery();
  const query = useQuery({
    ...dispatchWorkspacesQuery(identity.data?.pubkey),
    enabled: Boolean(identity.data?.pubkey),
  });
  return {
    workspaces: query.data ?? [],
    isLoading: identity.isLoading || query.isLoading,
    error: (identity.error ?? query.error) as Error | null,
    refresh: () => void query.refetch(),
  };
}

export function useDispatchContext(workspaceId: string | null) {
  const identity = useIdentityQuery();
  const queryClient = useQueryClient();
  const [liveError, setLiveError] = React.useState<Error | null>(null);
  const [relatedError, setRelatedError] = React.useState<Error | null>(null);
  const [isRetrying, setIsRetrying] = React.useState(false);
  const queryKey = [
    "dispatch",
    "context",
    workspaceId,
    identity.data?.pubkey,
  ] as const;
  const query = useQuery({
    ...dispatchContextQuery(
      workspaceId ?? "",
      identity.data?.pubkey ?? "",
      queryClient,
      setIsRetrying,
    ),
    enabled: Boolean(workspaceId && identity.data?.pubkey),
  });
  const preview = useQuery<DispatchContext>({
    queryKey: [...queryKey, "preview"],
    enabled: false,
  });
  const liveMergeRef = React.useRef<DispatchLiveMerge | null>(null);
  React.useEffect(() => {
    setLiveError(null);
    setIsRetrying(false);
    const managedSigner = identity.data?.pubkey;
    if (!workspaceId || !managedSigner) return;
    const activeQueryKey = [
      "dispatch",
      "context",
      workspaceId,
      managedSigner,
    ] as const;
    const disposers = new Set<() => void>();
    let cancelled = false;
    const merger = createDispatchLiveMerge({
      delayMs: LIVE_REFRESH_DEBOUNCE_MS,
      fold: ({ events, taskEvents, snapshotAt }) =>
        foldTrackerContextIncremental<Omit<DispatchContext, "sectionErrors">>({
          events,
          taskEvents,
          workspaceId,
          managedSigner,
          snapshotAt,
        }),
      onFolded: (folded) => {
        setLiveError(null);
        queryClient.setQueryData<DispatchContext>(
          activeQueryKey,
          (current) => ({
            ...folded,
            sectionErrors: current?.sectionErrors ?? {
              comments: null,
              projects: null,
              tasks: null,
            },
          }),
        );
      },
      onError: (error) => setLiveError(asError(error)),
    });
    merger.setColdLoadReady(Boolean(queryClient.getQueryData(activeQueryKey)));
    liveMergeRef.current = merger;
    const capture = (subscription: Promise<() => void>) => {
      void subscription
        .then((dispose) => {
          if (cancelled) dispose();
          else disposers.add(dispose);
        })
        .catch((error: unknown) => {
          if (!cancelled) setLiveError(asError(error));
        });
    };
    capture(
      relayClient.subscribeLive(
        { kinds: TRACKER_KINDS, "#h": [workspaceId], limit: 0 },
        merger.push,
      ),
    );
    capture(
      relayClient.subscribeLive(
        { kinds: [PROJECT_KIND], "#buzz-workspace": [workspaceId], limit: 0 },
        merger.push,
      ),
    );
    capture(
      relayClient.subscribeLive(
        {
          kinds: [TASK_KIND],
          "#t": [TASK_DISCOVERY_TAG],
          "#h": [workspaceId],
          limit: 0,
        },
        merger.push,
      ),
    );
    void relayClient
      .subscribeLive(
        { kinds: [40026, 40023, 40024, 40027], "#h": [workspaceId], limit: 0 },
        (event) => {
          merger.push(event);
          if (event.kind === 40027)
            void loadAdmissionSources(
              dispatchHistoryClient(),
              [event],
              workspaceId,
              new Set(),
            )
              .then((values) => {
                if (!cancelled) values.forEach(merger.push);
              })
              .catch((error) => {
                if (!cancelled) setLiveError(asError(error));
              });
        },
      )
      .then((dispose) => {
        if (cancelled) dispose();
        else disposers.add(dispose);
      })
      .catch((error: unknown) => {
        if (!cancelled)
          queryClient.setQueryData<DispatchContext>(
            activeQueryKey,
            (current) =>
              current
                ? {
                    ...current,
                    sectionErrors: {
                      ...current.sectionErrors,
                      records: asError(error).message,
                    },
                  }
                : current,
          );
      });
    return () => {
      cancelled = true;
      merger.cancel();
      if (liveMergeRef.current === merger) liveMergeRef.current = null;
      disposers.forEach((dispose) => {
        dispose();
      });
      disposers.clear();
    };
  }, [identity.data?.pubkey, queryClient, workspaceId]);
  React.useEffect(() => {
    liveMergeRef.current?.setColdLoadReady(Boolean(query.data));
  }, [query.data]);
  const relatedKey = JSON.stringify(
    linkedTaskReferences(query.data?.issues ?? []),
  );
  const relatedIssuesRef = React.useRef(query.data?.issues ?? []);
  relatedIssuesRef.current = query.data?.issues ?? [];
  React.useEffect(() => {
    setRelatedError(null);
    const managedSigner = identity.data?.pubkey;
    const merger = liveMergeRef.current;
    if (!workspaceId || !managedSigner || !merger) return;
    const refs = JSON.parse(relatedKey) as ReturnType<
      typeof linkedTaskReferences
    >;
    if (!refs.taskIds.length && !refs.eventIds.length) return;
    let cancelled = false;
    const disposers: (() => void)[] = [];
    const history = dispatchHistoryClient();
    const issues = relatedIssuesRef.current;
    const activeContextKey = [
      "dispatch",
      "context",
      workspaceId,
      managedSigner,
    ] as const;
    const start = async () => {
      const initial =
        queryClient.getQueryData<DispatchContext>(activeContextKey)
          ?.linkedTaskScope ??
        (await loadLinkedTaskHistory(
          history,
          issues,
          workspaceId,
          managedSigner,
        ));
      if (cancelled) return;
      for (const channelId of initial.channelIds) {
        for (let index = 0; index < initial.taskIds.length; index += 50) {
          const dispose = await relayClient.subscribeLive(
            {
              kinds: [TASK_KIND],
              "#h": [channelId],
              "#task": initial.taskIds.slice(index, index + 50),
              "#t": [TASK_DISCOVERY_TAG],
              limit: 0,
            },
            (event) => {
              if (!cancelled) merger.push(event);
            },
          );
          if (cancelled) {
            dispose();
            return;
          }
          disposers.push(dispose);
        }
        const disposeAdmission = await relayClient.subscribeLive(
          { kinds: [40027], "#h": [channelId], limit: 0 },
          (event) => {
            if (cancelled) return;
            merger.push(event);
            void loadAdmissionSources(history, [event], channelId, new Set())
              .then((sources) => {
                if (!cancelled) sources.forEach(merger.push);
              })
              .catch((error: unknown) => {
                if (!cancelled) setRelatedError(asError(error));
              });
          },
        );
        if (cancelled) {
          disposeAdmission();
          return;
        }
        disposers.push(disposeAdmission);
        const disposeMembership = await relayClient.subscribeLive(
          { kinds: [39002], "#d": [channelId], limit: 0 },
          (event) => {
            if (!cancelled) merger.push(event);
          },
        );
        if (cancelled) {
          disposeMembership();
          return;
        }
        disposers.push(disposeMembership);
      }
      // Install live readers before the final backfill so a transition cannot
      // disappear between history completion and subscription registration.
      const complete = await loadLinkedTaskHistory(
        history,
        issues,
        workspaceId,
        managedSigner,
      );
      if (cancelled) return;
      for (const event of [...complete.events, ...complete.taskEvents])
        merger.push(event);
    };
    void start().catch((error: unknown) => {
      if (!cancelled) setRelatedError(asError(error));
    });
    return () => {
      cancelled = true;
      disposers.forEach((dispose) => {
        dispose();
      });
    };
  }, [identity.data?.pubkey, relatedKey, workspaceId, queryClient]);
  const data = query.data ??
    preview.data ?? {
      issues: [],
      activity: [],
      comments: [],
      linkedTasks: [],
      projects: [],
      diagnostics: [],
      taskDiagnostics: [],
      viewerRole: "none" as const,
      sectionErrors: { comments: null, projects: null, tasks: null },
      snapshotAt: new Date(0).toISOString(),
    };
  return {
    ...data,
    sectionErrors: {
      ...data.sectionErrors,
      tasks: relatedError?.message ?? data.sectionErrors.tasks,
    },
    isRetrying,
    isLoading: identity.isLoading || (query.isLoading && !preview.data),
    isRefreshing: query.isFetching,
    isPreview: !query.data && Boolean(preview.data),
    error: (identity.error ?? query.error ?? liveError) as Error | null,
    refresh: async () => {
      const result = await query.refetch({ cancelRefetch: false });
      if (result.error) throw result.error;
      return result.data ?? data;
    },
  };
}
