import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import {
  getAgentUsageSeries,
  onAgentMetricsChanged,
  type AgentUsageSeries,
} from "@/shared/api/tauriArchive";

import { areBoundariesValid, lastNDaysBoundaries } from "./dayBoundaries";

export const agentUsageSeriesQueryKey = (boundaries: number[]) =>
  [
    "agentUsageSeries",
    boundaries[0],
    boundaries[boundaries.length - 1],
    boundaries.length,
  ] as const;

/**
 * Read the locally archived NIP-AM usage series for the last `days` local days.
 *
 * Invalidated by `onAgentMetricsChanged` rather than polled: the archive sync
 * task emits `archive-agent-metrics-changed` when it persists new kind-44200
 * rows, and `useArchiveAgentMetricsBridge` forwards it into that notifier.
 */
export function useAgentUsageSeries(days: number, enabled = true) {
  const queryClient = useQueryClient();
  // Recomputed only when `days` changes, so an intra-day re-render does not
  // rebuild the key and refetch. The window advances on the next remount or on
  // a metrics-changed invalidation, which is the same cadence the data moves.
  const boundaries = React.useMemo(() => lastNDaysBoundaries(days), [days]);
  const queryKey = agentUsageSeriesQueryKey(boundaries);

  React.useEffect(() => {
    if (!enabled) return;
    return onAgentMetricsChanged(() => {
      void queryClient.invalidateQueries({ queryKey });
    });
    // `queryKey` is a fresh array each render but its members are stable for a
    // given `days`; depend on those rather than the array identity.
  }, [enabled, queryClient, queryKey[1], queryKey[2], queryKey[3]]);

  return useQuery<AgentUsageSeries>({
    // A clock that jumped backwards mid-session can produce a non-increasing
    // boundary list. Fail fast here rather than surfacing the Rust validator's
    // error string to the user.
    enabled: enabled && areBoundariesValid(boundaries),
    queryKey,
    queryFn: () => getAgentUsageSeries({ bucketBoundaries: boundaries }),
    staleTime: 30_000,
  });
}
