import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useIdentityQuery } from "@/shared/api/hooks";
import { relayClient } from "@/shared/api/relayClient";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import { foldTrackerContext } from "@/shared/api/tauri";
import { createTrailingDebounce } from "@/shared/lib/trailingDebounce";
import type { RelayEvent } from "@/shared/api/types";
import type { DispatchContext, DispatchWorkspace } from "./types";

const PAGE_SIZE = 500;
const TRACKER_KINDS = [40020, 40021, 40022, 40025];
const PROJECT_KIND = 30621;
const LIVE_REFRESH_DEBOUNCE_MS = 250;

function asError(value: unknown): Error {
  return value instanceof Error
    ? value
    : new Error(`Dispatch live subscription failed: ${String(value)}`);
}

async function fetchExhaustively(
  filter: Omit<RelaySubscriptionFilter, "limit" | "until">,
): Promise<RelayEvent[]> {
  const events = new Map<string, RelayEvent>();
  let until: number | undefined;
  for (;;) {
    const page = await relayClient.fetchEvents({
      ...filter,
      limit: PAGE_SIZE,
      ...(until === undefined ? {} : { until }),
    });
    for (const event of page) events.set(event.id, event);
    if (page.length < PAGE_SIZE) return [...events.values()];
    const oldest = Math.min(...page.map((event) => event.created_at));
    const boundary = await relayClient.fetchEvents({
      ...filter,
      limit: PAGE_SIZE,
      since: oldest,
      until: oldest,
    });
    for (const event of boundary) events.set(event.id, event);
    if (boundary.length >= PAGE_SIZE) {
      throw new Error(
        "Tracker history cannot be exhausted at the timestamp boundary.",
      );
    }
    if (oldest <= 0) return [...events.values()];
    until = oldest - 1;
  }
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
  const memberships = await fetchExhaustively({
    kinds: [39002],
    "#p": [pubkey],
  });
  const ids = [
    ...new Set(memberships.map((event) => tag(event, "d")).filter(Boolean)),
  ] as string[];
  if (ids.length === 0) return [];
  const metadata = await fetchExhaustively({ kinds: [39000], "#d": ids });
  return Promise.all(
    ids.map(async (id) => {
      const probe = await relayClient.fetchEvents({
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
): Promise<DispatchContext> {
  const snapshotAt = Math.floor(Date.now() / 1000);
  const [memberships, trackerEvents, projectEvents] = await Promise.all([
    relayClient.fetchEvents({ kinds: [39002], "#d": [workspaceId], limit: 1 }),
    fetchExhaustively({ kinds: TRACKER_KINDS, "#h": [workspaceId] }),
    fetchExhaustively({
      kinds: [PROJECT_KIND],
      "#buzz-workspace": [workspaceId],
    }),
  ]);
  const membershipEvent = memberships[0];
  if (!membershipEvent)
    throw new Error("Tracker workspace membership snapshot is unavailable.");
  return foldTrackerContext<DispatchContext>({
    events: [...trackerEvents, ...projectEvents],
    membershipEvent,
    workspaceId,
    managedSigner,
    snapshotAt,
  });
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
  const [revision, setRevision] = React.useState(0);
  const [liveError, setLiveError] = React.useState<Error | null>(null);
  const query = useQuery({
    queryKey: [
      "dispatch",
      "context",
      workspaceId,
      identity.data?.pubkey,
      revision,
    ],
    queryFn: () => loadContext(workspaceId ?? "", identity.data?.pubkey ?? ""),
    enabled: Boolean(workspaceId && identity.data?.pubkey),
  });
  React.useEffect(() => {
    setLiveError(null);
    if (!workspaceId) return;
    const disposers = new Set<() => void>();
    let cancelled = false;
    const refresh = createTrailingDebounce(
      () => setRevision((value) => value + 1),
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
    projects: [],
    diagnostics: [],
    snapshotAt: new Date(0).toISOString(),
  };
  return {
    ...data,
    isLoading: identity.isLoading || query.isLoading,
    error: (identity.error ?? query.error ?? liveError) as Error | null,
    refresh: () => void query.refetch(),
  };
}
