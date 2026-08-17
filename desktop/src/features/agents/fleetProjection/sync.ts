import * as React from "react";

import { useFleetVisibilitySettings } from "@/features/agents/lib/useFleetVisibilitySettings";
import { relayClient } from "@/shared/api/relayClient";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { useFeatureEnabled } from "@/shared/features";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { FLEET_PROJECTION_EVENT_KINDS, fleetProjectionCache } from "./store";

const AUTHOR_CHUNK_SIZE = 100;
const BACKFILL_LIMIT = 2_000;

export function membershipScopedFleetOwners(
  currentPubkey: string,
  channels: readonly Channel[],
): string[] {
  const owners = new Set([normalizePubkey(currentPubkey)]);
  for (const channel of channels) {
    if (!channel.isMember || channel.archivedAt !== null) continue;
    for (const pubkey of channel.memberPubkeys) {
      const normalized = normalizePubkey(pubkey);
      if (normalized) owners.add(normalized);
    }
  }
  return [...owners].sort();
}

function chunks<T>(items: readonly T[], size: number): T[][] {
  const output: T[][] = [];
  for (let index = 0; index < items.length; index += size) {
    output.push(items.slice(index, index + size));
  }
  return output;
}

export function useFleetProjectionSync({
  channels,
  ownerPubkey,
  relayUrl,
}: {
  channels: readonly Channel[];
  ownerPubkey?: string;
  relayUrl?: string;
}) {
  const experimentEnabled = useFeatureEnabled("fleetVisibility");
  const settings = useFleetVisibilitySettings();
  const enabled = experimentEnabled && settings?.enabled === true;
  const ownerPubkeys = React.useMemo(
    () =>
      enabled && ownerPubkey
        ? membershipScopedFleetOwners(ownerPubkey, channels)
        : [],
    [channels, enabled, ownerPubkey],
  );
  React.useEffect(() => {
    if (!enabled || !ownerPubkey || !relayUrl || ownerPubkeys.length === 0) {
      fleetProjectionCache.reset([], false);
      return;
    }

    let cancelled = false;
    const disposers: Array<() => Promise<void>> = [];
    const authorChunks = chunks(ownerPubkeys, AUTHOR_CHUNK_SIZE);
    fleetProjectionCache.reset(ownerPubkeys, true);

    const onEvent = (event: RelayEvent) => {
      if (!cancelled) fleetProjectionCache.ingest(event);
    };

    void (async () => {
      try {
        await Promise.all(
          authorChunks.map(async (authors) => {
            const dispose = await relayClient.subscribeLive(
              {
                kinds: [...FLEET_PROJECTION_EVENT_KINDS],
                authors,
                limit: 0,
              },
              onEvent,
            );
            if (cancelled) {
              await dispose();
            } else {
              disposers.push(dispose);
            }
          }),
        );
        const pages = await Promise.all(
          authorChunks.map((authors) =>
            relayClient.fetchEvents({
              kinds: [...FLEET_PROJECTION_EVENT_KINDS],
              authors,
              limit: BACKFILL_LIMIT,
            }),
          ),
        );
        if (!cancelled) {
          fleetProjectionCache.ingestMany(pages.flat());
          fleetProjectionCache.finishLoading();
        }
      } catch (error) {
        if (!cancelled) {
          fleetProjectionCache.setError(
            error instanceof Error
              ? error.message
              : "Fleet projection sync failed.",
          );
        }
      }
    })();

    return () => {
      cancelled = true;
      for (const dispose of disposers) void dispose();
    };
  }, [enabled, ownerPubkey, ownerPubkeys, relayUrl]);
}

export function useFleetProjectionSnapshot() {
  return React.useSyncExternalStore(
    fleetProjectionCache.subscribe,
    fleetProjectionCache.getSnapshot,
    fleetProjectionCache.getSnapshot,
  );
}
