import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useIdentityQuery } from "@/shared/api/hooks";
import { relayClient } from "@/shared/api/relayClient";
import {
  activateRateLimit,
  isRateLimited,
  parseRateLimitHint,
  waitForRateLimit,
} from "@/shared/api/relayRateLimitGate";
import { foldTrackerContext } from "@/shared/api/tauri";
import { createTrailingDebounce } from "@/shared/lib/trailingDebounce";
import type { RelayEvent } from "@/shared/api/types";
import { createDispatchHistoryClient } from "./history";
import type { DispatchContext, DispatchWorkspace } from "./types";

const TRACKER_KINDS = [40020, 40021, 40022, 40025];
const TRACKER_HISTORY_KINDS = [40020, 40021];
const TRACKER_COMMENT_KIND = 40022;
const PROJECT_KIND = 30621;
const TASK_KIND = 9;
const TASK_DISCOVERY_TAG = "buzz-task";
const LIVE_REFRESH_DEBOUNCE_MS = 250;

function asError(value: unknown): Error {
  return value instanceof Error
    ? value
    : new Error(`Dispatch live subscription failed: ${String(value)}`);
}

function dispatchHistoryClient(onRetryingChange?: (value: boolean) => void) {
  return createDispatchHistoryClient({
    fetchEvents: (filter) => relayClient.fetchEvents(filter),
    waitForRateLimit,
    isRateLimited,
    activateRateLimit: (message) =>
      activateRateLimit(parseRateLimitHint(message)),
    onRetryingChange,
  });
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
  return Promise.all(
    ids.map(async (id) => {
      const probe = await history.fetch({
        kinds: TRACKER_KINDS,
        "#h": [id],
        limit: 1,
      });
      const meta = metadata
        .filter((event) => tag(event, "d") === id)
        .sort((a, b) => b.created_at - a.created_at)[0];
      return {
        id,
        name: workspaceName(meta, id),
        trackerEventCount: probe.length,
      };
    }),
  );
}

async function loadContext(
  workspaceId: string,
  managedSigner: string,
  onRetryingChange: (value: boolean) => void,
): Promise<DispatchContext> {
  const history = dispatchHistoryClient(onRetryingChange);
  const snapshotAt = Math.floor(Date.now() / 1000);
  const memberships = await history.fetch({
    kinds: [39002],
    "#d": [workspaceId],
    limit: 1,
  });
  const [trackerHistory, comments, projects, tasks] = await Promise.allSettled([
    history.fetchExhaustively({
      kinds: TRACKER_HISTORY_KINDS,
      "#h": [workspaceId],
    }),
    history.fetchExhaustively({
      kinds: [TRACKER_COMMENT_KIND],
      "#h": [workspaceId],
    }),
    history.fetchExhaustively({
      kinds: [PROJECT_KIND],
      "#buzz-workspace": [workspaceId],
    }),
    history.fetchExhaustively({
      kinds: [TASK_KIND],
      "#t": [TASK_DISCOVERY_TAG],
      "#h": [workspaceId],
    }),
  ]);
  if (trackerHistory.status === "rejected") throw trackerHistory.reason;
  const membershipEvent = memberships[0];
  if (!membershipEvent)
    throw new Error("Tracker workspace membership snapshot is unavailable.");
  const folded = await foldTrackerContext<
    Omit<DispatchContext, "sectionErrors">
  >({
    events: [
      ...trackerHistory.value,
      ...(comments.status === "fulfilled" ? comments.value : []),
      ...(projects.status === "fulfilled" ? projects.value : []),
    ],
    taskEvents: tasks.status === "fulfilled" ? tasks.value : [],
    membershipEvent,
    workspaceId,
    managedSigner,
    snapshotAt,
  });
  const errorMessage = (result: PromiseSettledResult<unknown>) =>
    result.status === "rejected"
      ? result.reason instanceof Error
        ? result.reason.message
        : String(result.reason)
      : null;
  return {
    ...folded,
    sectionErrors: {
      comments: errorMessage(comments),
      projects: errorMessage(projects),
      tasks: errorMessage(tasks),
    },
  };
}

export function useDispatchWorkspaces() {
  const identity = useIdentityQuery();
  const query = useQuery({
    queryKey: ["dispatch", "workspaces", identity.data?.pubkey],
    queryFn: () => loadWorkspaces(identity.data?.pubkey ?? ""),
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
  const [liveError, setLiveError] = React.useState<Error | null>(null);
  const [isRetrying, setIsRetrying] = React.useState(false);
  const query = useQuery({
    queryKey: ["dispatch", "context", workspaceId, identity.data?.pubkey],
    queryFn: () =>
      loadContext(
        workspaceId ?? "",
        identity.data?.pubkey ?? "",
        setIsRetrying,
      ),
    enabled: Boolean(workspaceId && identity.data?.pubkey),
  });
  const refetchRef = React.useRef(query.refetch);
  refetchRef.current = query.refetch;
  React.useEffect(() => {
    setLiveError(null);
    setIsRetrying(false);
    if (!workspaceId) return;
    const disposers = new Set<() => void>();
    let cancelled = false;
    const refresh = createTrailingDebounce(
      () => void refetchRef.current({ cancelRefetch: false }),
      LIVE_REFRESH_DEBOUNCE_MS,
    );
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
        refresh.trigger,
      ),
    );
    capture(
      relayClient.subscribeLive(
        { kinds: [PROJECT_KIND], "#buzz-workspace": [workspaceId], limit: 0 },
        refresh.trigger,
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
        refresh.trigger,
      ),
    );
    return () => {
      cancelled = true;
      refresh.cancel();
      disposers.forEach((dispose) => {
        dispose();
      });
      disposers.clear();
    };
  }, [workspaceId]);
  const data = query.data ?? {
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
    isRetrying,
    isLoading: identity.isLoading || query.isLoading,
    error: (identity.error ?? query.error ?? liveError) as Error | null,
    refresh: async () => {
      const result = await query.refetch({ cancelRefetch: false });
      if (result.error) throw result.error;
      return result.data ?? data;
    },
  };
}
