import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_DELETION,
  KIND_HOST_DECLARATION,
  KIND_HOST_STATUS,
  KIND_MANAGED_AGENT,
  KIND_PERSONA,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type {
  FleetProjectionAgent,
  FleetProjectionAgentStatus,
  FleetProjectionHost,
  FleetProjectionLifecycleState,
  FleetProjectionOwner,
  FleetProjectionRespondTo,
  FleetProjectionSnapshot,
} from "./types";

export const FLEET_PROJECTION_EVENT_KINDS = [
  KIND_DELETION,
  KIND_PERSONA,
  KIND_MANAGED_AGENT,
  KIND_HOST_DECLARATION,
  KIND_HOST_STATUS,
] as const;

const EMPTY_SNAPSHOT: FleetProjectionSnapshot = {
  owners: [],
  loading: false,
  errorMessage: null,
};
const PUBKEY_PATTERN = /^[0-9a-f]{64}$/;
const HOST_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const CONTROL_CHARACTER_PATTERN = /\p{Cc}/u;
const HOST_DECLARATION_MAX_BYTES = 4_096;
const HOST_STATUS_MAX_BYTES = 65_536;
const HOST_STATUS_MAX_AGENTS = 256;

type Version = {
  createdAt: number;
  eventId: string;
};

type AgentHead = Version & {
  ownerPubkey: string;
  agentPubkey: string;
  name: string;
  personaId: string | null;
  respondTo: FleetProjectionRespondTo;
  respondToAllowlist: readonly string[];
};

type PersonaHead = Version & {
  ownerPubkey: string;
  personaId: string;
  displayName: string;
  avatarUrl: string | null;
};

type HostDeclarationHead = Version & {
  ownerPubkey: string;
  hostId: string;
  name: string;
  platform: "macos" | "linux" | "windows";
  declaredAt: number;
};

type HostStatusHead = Version & {
  ownerPubkey: string;
  hostId: string;
  seq: number;
  timestamp: number;
  agents: readonly FleetProjectionAgentStatus[];
};

type ProjectionHead =
  | AgentHead
  | PersonaHead
  | HostDeclarationHead
  | HostStatusHead;

type ParsedCoordinate = {
  kind: number;
  ownerPubkey: string;
  dTag: string;
};

function coordinate(kind: number, ownerPubkey: string, dTag: string) {
  return `${kind}:${normalizePubkey(ownerPubkey)}:${dTag}`;
}

function eventDTag(event: RelayEvent): string | null {
  const tags = event.tags.filter((tag) => tag[0] === "d");
  return tags.length === 1 && tags[0]?.[1] ? tags[0][1] : null;
}

function parseObject(content: string): Record<string, unknown> | null {
  try {
    const parsed: unknown = JSON.parse(content);
    return typeof parsed === "object" &&
      parsed !== null &&
      !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function hasExactKeys(
  record: Readonly<Record<string, unknown>>,
  keys: readonly string[],
) {
  const actual = Object.keys(record).sort();
  const expected = [...keys].sort();
  return (
    actual.length === expected.length &&
    actual.every((key, index) => key === expected[index])
  );
}

function validPubkey(value: unknown): value is string {
  return typeof value === "string" && PUBKEY_PATTERN.test(value);
}

function validRespondTo(value: unknown): value is FleetProjectionRespondTo {
  return value === "owner-only" || value === "allowlist" || value === "anyone";
}

function validLifecycleState(
  value: unknown,
): value is FleetProjectionLifecycleState {
  return value === "running" || value === "stopped" || value === "error";
}

function parseAgentHead(event: RelayEvent): AgentHead | null {
  const agentPubkey = eventDTag(event);
  const content = parseObject(event.content);
  if (!agentPubkey || !validPubkey(agentPubkey) || !content) return null;

  const name = typeof content.name === "string" ? content.name.trim() : "";
  const personaId =
    typeof content.persona_id === "string" && content.persona_id.trim()
      ? content.persona_id.trim()
      : null;
  const respondTo = content.respond_to;
  if (!name || !validRespondTo(respondTo)) return null;

  const allowlist = Array.isArray(content.respond_to_allowlist)
    ? content.respond_to_allowlist.filter(validPubkey).map(normalizePubkey)
    : [];
  if (
    Array.isArray(content.respond_to_allowlist) &&
    allowlist.length !== content.respond_to_allowlist.length
  ) {
    return null;
  }

  return {
    ownerPubkey: normalizePubkey(event.pubkey),
    agentPubkey: normalizePubkey(agentPubkey),
    name,
    personaId,
    respondTo,
    respondToAllowlist: allowlist,
    createdAt: event.created_at,
    eventId: event.id,
  };
}

function parsePersonaHead(event: RelayEvent): PersonaHead | null {
  const personaId = eventDTag(event);
  const content = parseObject(event.content);
  if (!personaId || !content) return null;
  const displayName =
    typeof content.display_name === "string" ? content.display_name.trim() : "";
  const avatarUrl =
    typeof content.avatar_url === "string" && content.avatar_url.trim()
      ? content.avatar_url.trim()
      : null;
  if (!displayName) return null;
  return {
    ownerPubkey: normalizePubkey(event.pubkey),
    personaId,
    displayName,
    avatarUrl,
    createdAt: event.created_at,
    eventId: event.id,
  };
}

function parseHostDeclarationHead(
  event: RelayEvent,
): HostDeclarationHead | null {
  const hostId = eventDTag(event);
  const content = parseObject(event.content);
  if (
    !hostId ||
    !HOST_ID_PATTERN.test(hostId) ||
    !content ||
    new TextEncoder().encode(event.content).byteLength >
      HOST_DECLARATION_MAX_BYTES ||
    !hasExactKeys(content, ["v", "name", "platform", "created_at"])
  ) {
    return null;
  }
  const name = typeof content.name === "string" ? content.name.trim() : "";
  const platform = content.platform;
  const declaredAt = content.created_at;
  if (
    content.v !== 1 ||
    !name ||
    name !== content.name ||
    [...name].length > 128 ||
    CONTROL_CHARACTER_PATTERN.test(name) ||
    (platform !== "macos" && platform !== "linux" && platform !== "windows") ||
    !Number.isSafeInteger(declaredAt) ||
    (declaredAt as number) <= 0
  ) {
    return null;
  }
  return {
    ownerPubkey: normalizePubkey(event.pubkey),
    hostId,
    name,
    platform,
    declaredAt: declaredAt as number,
    createdAt: event.created_at,
    eventId: event.id,
  };
}

function parseAgentStatus(value: unknown): FleetProjectionAgentStatus | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  if (
    !hasExactKeys(record, ["pubkey", "state", "since"]) ||
    !validPubkey(record.pubkey) ||
    !validLifecycleState(record.state) ||
    !Number.isSafeInteger(record.since) ||
    (record.since as number) <= 0
  ) {
    return null;
  }
  return {
    pubkey: normalizePubkey(record.pubkey),
    state: record.state,
    since: record.since as number,
  };
}

function parseHostStatusHead(event: RelayEvent): HostStatusHead | null {
  const hostId = eventDTag(event);
  const content = parseObject(event.content);
  if (
    !hostId ||
    !HOST_ID_PATTERN.test(hostId) ||
    !content ||
    new TextEncoder().encode(event.content).byteLength >
      HOST_STATUS_MAX_BYTES ||
    !hasExactKeys(content, ["v", "seq", "ts", "agents"])
  ) {
    return null;
  }
  const seq = content.seq;
  const timestamp = content.ts;
  if (
    content.v !== 1 ||
    !Number.isSafeInteger(seq) ||
    (seq as number) <= 0 ||
    !Number.isSafeInteger(timestamp) ||
    (timestamp as number) <= 0 ||
    !Array.isArray(content.agents) ||
    content.agents.length > HOST_STATUS_MAX_AGENTS ||
    Math.abs((timestamp as number) - event.created_at) > 900
  ) {
    return null;
  }
  const agents = content.agents.map(parseAgentStatus);
  if (agents.some((agent) => agent === null)) return null;
  const typedAgents = agents as FleetProjectionAgentStatus[];
  if (
    new Set(typedAgents.map((agent) => agent.pubkey)).size !==
      typedAgents.length ||
    typedAgents.some((agent) => agent.since > (timestamp as number))
  ) {
    return null;
  }
  return {
    ownerPubkey: normalizePubkey(event.pubkey),
    hostId,
    seq: seq as number,
    timestamp: timestamp as number,
    agents: typedAgents,
    createdAt: event.created_at,
    eventId: event.id,
  };
}

function parseProjectionHead(event: RelayEvent): ProjectionHead | null {
  switch (event.kind) {
    case KIND_MANAGED_AGENT:
      return parseAgentHead(event);
    case KIND_PERSONA:
      return parsePersonaHead(event);
    case KIND_HOST_DECLARATION:
      return parseHostDeclarationHead(event);
    case KIND_HOST_STATUS:
      return parseHostStatusHead(event);
    default:
      return null;
  }
}

function parseDeletionCoordinates(event: RelayEvent): ParsedCoordinate[] {
  if (event.kind !== KIND_DELETION) return [];
  const author = normalizePubkey(event.pubkey);
  return event.tags.flatMap((tag) => {
    if (tag[0] !== "a" || !tag[1]) return [];
    const [kindText, ownerPubkey, dTag] = tag[1].split(":", 3);
    const kind = Number(kindText);
    if (
      !dTag ||
      normalizePubkey(ownerPubkey ?? "") !== author ||
      ![
        KIND_PERSONA,
        KIND_MANAGED_AGENT,
        KIND_HOST_DECLARATION,
        KIND_HOST_STATUS,
      ].includes(kind)
    ) {
      return [];
    }
    return [{ kind, ownerPubkey: author, dTag }];
  });
}

function versionWins(candidate: Version, current: Version | undefined) {
  if (!current) return true;
  if (candidate.createdAt !== current.createdAt) {
    return candidate.createdAt > current.createdAt;
  }
  return candidate.eventId < current.eventId;
}

function buildSnapshot(
  heads: ReadonlyMap<string, ProjectionHead>,
  loading: boolean,
  errorMessage: string | null,
): FleetProjectionSnapshot {
  const agentsByOwner = new Map<string, AgentHead[]>();
  const personas = new Map<string, PersonaHead>();
  const declarations = new Map<string, HostDeclarationHead>();
  const statuses = new Map<string, HostStatusHead>();

  for (const head of heads.values()) {
    if ("agentPubkey" in head) {
      const current = agentsByOwner.get(head.ownerPubkey) ?? [];
      current.push(head);
      agentsByOwner.set(head.ownerPubkey, current);
    } else if ("personaId" in head) {
      personas.set(
        coordinate(KIND_PERSONA, head.ownerPubkey, head.personaId),
        head,
      );
    } else if ("declaredAt" in head) {
      declarations.set(
        coordinate(KIND_HOST_DECLARATION, head.ownerPubkey, head.hostId),
        head,
      );
    } else {
      statuses.set(
        coordinate(KIND_HOST_STATUS, head.ownerPubkey, head.hostId),
        head,
      );
    }
  }

  const ownerPubkeys = new Set<string>();
  for (const owner of agentsByOwner.keys()) ownerPubkeys.add(owner);
  for (const declaration of declarations.values())
    ownerPubkeys.add(declaration.ownerPubkey);
  for (const status of statuses.values()) ownerPubkeys.add(status.ownerPubkey);

  const owners: FleetProjectionOwner[] = [...ownerPubkeys]
    .sort()
    .map((ownerPubkey) => {
      const agents: FleetProjectionAgent[] = (
        agentsByOwner.get(ownerPubkey) ?? []
      )
        .sort((left, right) =>
          left.name.localeCompare(right.name, undefined, {
            sensitivity: "base",
          }),
        )
        .map((agent) => {
          const persona = agent.personaId
            ? personas.get(
                coordinate(KIND_PERSONA, ownerPubkey, agent.personaId),
              )
            : null;
          return {
            ownerPubkey,
            agentPubkey: agent.agentPubkey,
            name: agent.name,
            personaId: agent.personaId,
            personaDisplayName: persona?.displayName ?? null,
            personaAvatarUrl: persona?.avatarUrl ?? null,
            respondTo: agent.respondTo,
            respondToAllowlist: agent.respondToAllowlist,
          };
        });

      const hostIds = new Set<string>();
      for (const declaration of declarations.values()) {
        if (declaration.ownerPubkey === ownerPubkey)
          hostIds.add(declaration.hostId);
      }
      for (const status of statuses.values()) {
        if (status.ownerPubkey === ownerPubkey) hostIds.add(status.hostId);
      }
      const hosts: FleetProjectionHost[] = [...hostIds]
        .map((hostId) => {
          const declaration = declarations.get(
            coordinate(KIND_HOST_DECLARATION, ownerPubkey, hostId),
          );
          const status = statuses.get(
            coordinate(KIND_HOST_STATUS, ownerPubkey, hostId),
          );
          return {
            ownerPubkey,
            hostId,
            name: declaration?.name ?? null,
            platform: declaration?.platform ?? null,
            declaredAt: declaration?.declaredAt ?? null,
            status: status
              ? {
                  seq: status.seq,
                  timestamp: status.timestamp,
                  agents: status.agents,
                }
              : null,
          };
        })
        .sort((left, right) => {
          const time =
            (right.status?.timestamp ?? 0) - (left.status?.timestamp ?? 0);
          return time !== 0 ? time : left.hostId.localeCompare(right.hostId);
        });

      return { ownerPubkey, agents, hosts };
    });

  return { owners, loading, errorMessage };
}

export type FleetProjectionCache = ReturnType<
  typeof createFleetProjectionCache
>;

export function createFleetProjectionCache() {
  const heads = new Map<string, ProjectionHead>();
  const tombstones = new Map<string, Version>();
  const listeners = new Set<() => void>();
  let snapshot = EMPTY_SNAPSHOT;
  let allowedOwners = new Set<string>();

  const publish = (
    loading = snapshot.loading,
    errorMessage = snapshot.errorMessage,
  ) => {
    snapshot = buildSnapshot(heads, loading, errorMessage);
    for (const listener of listeners) listener();
  };

  const ingest = (event: RelayEvent): boolean => {
    const ownerPubkey = normalizePubkey(event.pubkey);
    if (!allowedOwners.has(ownerPubkey)) return false;

    if (event.kind === KIND_DELETION) {
      let changed = false;
      for (const target of parseDeletionCoordinates(event)) {
        const key = coordinate(target.kind, target.ownerPubkey, target.dTag);
        const marker: Version = {
          createdAt: event.created_at,
          eventId: event.id,
        };
        const currentTombstone = tombstones.get(key);
        if (!versionWins(marker, currentTombstone)) continue;
        tombstones.set(key, marker);
        const currentHead = heads.get(key);
        if (currentHead && event.created_at >= currentHead.createdAt) {
          heads.delete(key);
          changed = true;
        }
      }
      return changed;
    }

    const parsed = parseProjectionHead(event);
    const dTag = eventDTag(event);
    if (!parsed || !dTag) return false;
    const key = coordinate(event.kind, ownerPubkey, dTag);
    const tombstone = tombstones.get(key);
    if (tombstone && parsed.createdAt <= tombstone.createdAt) return false;
    const current = heads.get(key);
    if (!versionWins(parsed, current)) return false;
    heads.set(key, parsed);
    return true;
  };

  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    reset(ownerPubkeys: readonly string[], loading: boolean) {
      allowedOwners = new Set(ownerPubkeys.map(normalizePubkey));
      heads.clear();
      tombstones.clear();
      publish(loading, null);
    },
    ingest(event: RelayEvent) {
      if (ingest(event)) publish();
    },
    ingestMany(events: readonly RelayEvent[]) {
      let changed = false;
      for (const event of [...events].sort((left, right) => {
        const time = left.created_at - right.created_at;
        return time !== 0 ? time : right.id.localeCompare(left.id);
      })) {
        changed = ingest(event) || changed;
      }
      if (changed) publish();
    },
    finishLoading() {
      publish(false, null);
    },
    setError(message: string) {
      publish(false, message);
    },
  };
}

export const fleetProjectionCache = createFleetProjectionCache();
