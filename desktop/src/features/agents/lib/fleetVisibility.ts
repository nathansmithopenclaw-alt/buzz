import type { FleetVisibilitySettings } from "@/shared/api/tauriManagedAgents";
import type { RelayEvent } from "@/shared/api/types";
import type { ConnectionState } from "@/shared/api/relayClientShared";
import {
  KIND_HOST_DECLARATION,
  KIND_MANAGED_AGENT,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

type HostDeclaration = {
  v: 1;
  name: string;
  platform: "macos" | "linux" | "windows";
  created_at: number;
};

function dTag(event: RelayEvent): string | null {
  return event.tags.find((tag) => tag[0] === "d")?.[1] ?? null;
}

function parseHostName(event: RelayEvent): string | null {
  if (event.kind !== KIND_HOST_DECLARATION) return null;
  try {
    const content = JSON.parse(event.content) as Partial<HostDeclaration>;
    const name = typeof content.name === "string" ? content.name.trim() : "";
    return content.v === 1 && name ? name : null;
  } catch {
    return null;
  }
}

export type FleetVisibilityBootstrap = {
  unmatchedAgentPubkeys: string[];
  hostName: string | null;
};

export type FleetVisibilityBootstrapRead =
  | { kind: "auto-host" }
  | {
      kind: "needs-choice";
      detectionFailed: boolean;
      bootstrap: FleetVisibilityBootstrap;
      fleetRecordCount: number;
    };

/**
 * Detect owner projections that cannot be backed by a keyed local record.
 * This is only the first-run role chooser seam; FV-B2 owns the durable
 * projection cache and roster rendering.
 */
export function deriveFleetVisibilityBootstrap(
  events: readonly RelayEvent[],
  localAgentPubkeys: readonly string[],
  localHostId?: string | null,
): FleetVisibilityBootstrap {
  const local = new Set(localAgentPubkeys.map(normalizePubkey));
  const unmatchedAgentPubkeys = Array.from(
    new Set(
      events
        .filter((event) => event.kind === KIND_MANAGED_AGENT)
        .map(dTag)
        .filter((pubkey): pubkey is string => Boolean(pubkey))
        .map(normalizePubkey)
        .filter((pubkey) => pubkey.length === 64 && !local.has(pubkey)),
    ),
  ).sort();

  const hostName =
    [...events]
      .filter((event) => event.kind === KIND_HOST_DECLARATION)
      .filter((event) => dTag(event) !== localHostId)
      .sort((left, right) => right.created_at - left.created_at)
      .map(parseHostName)
      .find((name): name is string => Boolean(name)) ?? null;

  return { unmatchedAgentPubkeys, hostName };
}

/**
 * First-run role detection may auto-select Host only across a relay read that
 * started and finished on an authenticated connection and returned no fleet
 * records. A not-ready/error result is deliberately represented by `null` so
 * callers cannot confuse it with a successful empty EOSE.
 */
export function classifyFleetVisibilityBootstrapRead(
  startedConnectionState: ConnectionState,
  finishedConnectionState: ConnectionState,
  events: readonly RelayEvent[] | null,
  localAgentPubkeys: readonly string[],
  localHostId?: string | null,
): FleetVisibilityBootstrapRead {
  const authenticatedRead =
    startedConnectionState === "connected" &&
    finishedConnectionState === "connected" &&
    events !== null;

  if (authenticatedRead && events.length === 0) {
    return { kind: "auto-host" };
  }

  return {
    kind: "needs-choice",
    detectionFailed: !authenticatedRead,
    bootstrap: deriveFleetVisibilityBootstrap(
      events ?? [],
      localAgentPubkeys,
      localHostId,
    ),
    fleetRecordCount: events?.length ?? 0,
  };
}

/** UI-only defense; unknown backend state must not hide creation candidates. */
export function shouldBlockPersonaCreation(
  experimentEnabled: boolean,
  settings: FleetVisibilitySettings | null,
): boolean {
  return (
    experimentEnabled &&
    settings?.enabled === true &&
    (settings.needsChoice || settings.role === "observer")
  );
}
