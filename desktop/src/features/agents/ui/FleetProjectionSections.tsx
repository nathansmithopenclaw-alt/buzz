import * as React from "react";
import { CircleAlert, RadioTower } from "lucide-react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import {
  fleetAgentLiveness,
  fleetHostIsFresh,
  presentFleetProjectionAgent,
  primaryFleetHost,
} from "@/features/agents/fleetProjection/selectors";
import { useFleetProjectionSnapshot } from "@/features/agents/fleetProjection/sync";
import type {
  FleetProjectionAgentPresentation,
  FleetAgentLiveness,
} from "@/features/agents/fleetProjection/selectors";
import type {
  FleetProjectionHost,
  FleetProjectionOwner,
} from "@/features/agents/fleetProjection/types";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { IdentityInitialsAvatar } from "./IdentityInitialsAvatar";
import { IDENTITY_CARD_GRID_CLASS } from "./agentCardLayout";

function useFleetProjectionPresentation() {
  const snapshot = useFleetProjectionSnapshot();
  const pubkeys = React.useMemo(
    () => [
      ...new Set(
        snapshot.owners.flatMap((owner) => [
          owner.ownerPubkey,
          ...owner.agents.map((agent) => agent.agentPubkey),
        ]),
      ),
    ],
    [snapshot.owners],
  );
  const profilesQuery = useUsersBatchQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  });
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: pubkeys.length > 0 });
  const relayAgentsByPubkey = React.useMemo(
    () =>
      new Map(
        (relayAgentsQuery.data ?? []).map((agent) => [
          normalizePubkey(agent.pubkey),
          agent,
        ]),
      ),
    [relayAgentsQuery.data],
  );
  const presentedOwners = React.useMemo(
    () =>
      snapshot.owners.map((owner) => ({
        ...owner,
        agents: owner.agents.map((agent) =>
          presentFleetProjectionAgent(
            agent,
            profilesQuery.data?.profiles[normalizePubkey(agent.agentPubkey)],
            relayAgentsByPubkey.get(normalizePubkey(agent.agentPubkey)),
          ),
        ),
      })),
    [profilesQuery.data?.profiles, relayAgentsByPubkey, snapshot.owners],
  );

  return {
    ...snapshot,
    owners: presentedOwners,
    ownerProfiles: profilesQuery.data?.profiles ?? {},
  };
}

function useCurrentUnixSeconds() {
  const [now, setNow] = React.useState(() => Math.floor(Date.now() / 1_000));
  React.useEffect(() => {
    const interval = window.setInterval(
      () => setNow(Math.floor(Date.now() / 1_000)),
      1_000,
    );
    return () => window.clearInterval(interval);
  }, []);
  return now;
}

function formatRelative(timestamp: number, now: number) {
  const seconds = Math.max(0, now - timestamp);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

function hostLabel(host: FleetProjectionHost | null) {
  return host?.name?.trim() || "an unnamed host";
}

function FleetHostStrip({
  host,
  now,
}: {
  host: FleetProjectionHost | null;
  now: number;
}) {
  const fresh = fleetHostIsFresh(host, now);
  const message = host?.status
    ? fresh
      ? `heartbeat ${formatRelative(host.status.timestamp, now)}`
      : `unreachable since ${new Date(host.status.timestamp * 1_000).toLocaleString()}`
    : "status not reported";
  return (
    <div
      className="flex items-center gap-2 rounded-xl border border-border/70 bg-muted/35 px-4 py-3 text-sm"
      data-fresh={fresh ? "true" : "false"}
      data-testid="fleet-host-status"
    >
      <RadioTower
        className={
          fresh ? "h-4 w-4 text-emerald-500" : "h-4 w-4 text-muted-foreground"
        }
      />
      <span>
        Fleet hosted on <strong>{hostLabel(host)}</strong>
        <span className="text-muted-foreground"> · {message}</span>
      </span>
    </div>
  );
}

function livenessLabel(liveness: FleetAgentLiveness, now: number) {
  if (liveness.stale) return "Status unavailable · host unreachable";
  if (liveness.state === "unknown" || liveness.since === null) {
    return "Definition · status not reported";
  }
  return `${liveness.state[0]?.toUpperCase()}${liveness.state.slice(1)} · since ${formatRelative(liveness.since, now)}`;
}

function livenessDotClass(liveness: FleetAgentLiveness) {
  if (liveness.stale || liveness.state === "unknown") {
    return "bg-muted-foreground";
  }
  if (liveness.state === "running") return "bg-emerald-500";
  if (liveness.state === "error") return "bg-destructive";
  return "bg-slate-400";
}

function FleetAgentAvatar({
  agent,
  liveness,
  now,
}: {
  agent: FleetProjectionAgentPresentation;
  liveness: FleetAgentLiveness;
  now: number;
}) {
  return (
    <div className="relative h-24 w-24">
      {agent.avatarUrl ? (
        <ProfileAvatar
          avatarUrl={agent.avatarUrl}
          className="h-full w-full border-[3px] border-background bg-muted shadow-none"
          iconClassName="h-8 w-8"
          label={agent.displayName}
        />
      ) : (
        <IdentityInitialsAvatar
          className="shadow-none"
          label={agent.displayName}
          size={96}
        />
      )}
      <span
        aria-label={livenessLabel(liveness, now)}
        className={`absolute right-0 bottom-0 h-5 w-5 rounded-full border-[3px] border-background ${livenessDotClass(liveness)}`}
        data-testid={`fleet-agent-liveness-${agent.agentPubkey}`}
        role="img"
      />
    </div>
  );
}

function FleetAgentCards({
  agents,
  now,
  owner,
}: {
  agents: readonly FleetProjectionAgentPresentation[];
  now: number;
  owner: FleetProjectionOwner;
}) {
  if (agents.length === 0) {
    return (
      <p className="rounded-xl border border-dashed border-border/70 px-4 py-5 text-sm text-muted-foreground">
        No relay agent definitions found.
      </p>
    );
  }
  return (
    <div className={IDENTITY_CARD_GRID_CLASS}>
      {agents.map((agent) => {
        const liveness = fleetAgentLiveness(owner, agent.agentPubkey, now);
        return (
          <AgentIdentityCard
            ariaLabel={`${agent.displayName} agent profile`}
            avatar={
              <FleetAgentAvatar agent={agent} liveness={liveness} now={now} />
            }
            avatarUrl={agent.avatarUrl}
            dataTestId={`fleet-agent-${agent.agentPubkey}`}
            key={agent.agentPubkey}
            label={agent.displayName}
            modelLabel={livenessLabel(liveness, now)}
          />
        );
      })}
    </div>
  );
}

function ProjectionLoading() {
  return (
    <div
      className={IDENTITY_CARD_GRID_CLASS}
      data-testid="fleet-projection-loading"
    >
      <IdentityCardSkeleton
        footerSubtitleWidthClass="w-20"
        footerTitleWidthClass="w-24"
      />
      <IdentityCardSkeleton
        footerSubtitleWidthClass="w-16"
        footerTitleWidthClass="w-32"
      />
    </div>
  );
}

export function ObserverFleetSections({
  currentOwnerPubkey,
}: {
  currentOwnerPubkey: string;
}) {
  const projection = useFleetProjectionPresentation();
  const now = useCurrentUnixSeconds();
  const currentOwner = projection.owners.find(
    (owner) => owner.ownerPubkey === normalizePubkey(currentOwnerPubkey),
  );

  return (
    <div className="flex flex-col gap-8">
      <section className="space-y-4" data-testid="observer-my-agents">
        <div>
          <h2 className="text-base font-semibold">My agents</h2>
          <p className="text-sm text-muted-foreground">
            Relay definitions from your host. This device is read-only.
          </p>
        </div>
        <FleetHostStrip host={primaryFleetHost(currentOwner)} now={now} />
        {projection.loading ? <ProjectionLoading /> : null}
        {!projection.loading && currentOwner ? (
          <FleetAgentCards
            agents={currentOwner.agents}
            now={now}
            owner={currentOwner}
          />
        ) : null}
        {!projection.loading && !currentOwner ? (
          <p className="rounded-xl border border-dashed border-border/70 px-4 py-5 text-sm text-muted-foreground">
            No fleet projection is available for this owner yet.
          </p>
        ) : null}
        {projection.errorMessage ? (
          <p className="flex items-center gap-2 rounded-xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            <CircleAlert className="h-4 w-4" />
            {projection.errorMessage}
          </p>
        ) : null}
      </section>
      <FleetTeamSection currentOwnerPubkey={currentOwnerPubkey} />
    </div>
  );
}

export function FleetTeamSection({
  currentOwnerPubkey,
}: {
  currentOwnerPubkey: string;
}) {
  const projection = useFleetProjectionPresentation();
  const now = useCurrentUnixSeconds();
  const current = normalizePubkey(currentOwnerPubkey);
  const teamOwners = projection.owners.filter(
    (owner) => owner.ownerPubkey !== current && owner.agents.length > 0,
  );
  if (teamOwners.length === 0 && !projection.loading) return null;

  return (
    <section className="space-y-4" data-testid="fleet-team-section">
      <div>
        <h2 className="text-base font-semibold">Team</h2>
        <p className="text-sm text-muted-foreground">
          Read-only fleets from owners who share a channel with you.
        </p>
      </div>
      {projection.loading ? <ProjectionLoading /> : null}
      {teamOwners.map((owner) => {
        const profile = projection.ownerProfiles[owner.ownerPubkey];
        const ownerName =
          profile?.displayName?.trim() ||
          profile?.name?.trim() ||
          profile?.nip05Handle?.trim() ||
          truncatePubkey(owner.ownerPubkey);
        return (
          <div className="space-y-3" key={owner.ownerPubkey}>
            <div className="flex items-center justify-between gap-3">
              <div>
                <h3 className="text-sm font-medium">{ownerName}</h3>
                <p className="text-xs text-muted-foreground">
                  {owner.agents.length} agent
                  {owner.agents.length === 1 ? "" : "s"}
                </p>
              </div>
              <span className="rounded-full border border-border/70 px-2 py-1 text-2xs font-medium text-muted-foreground">
                Read-only
              </span>
            </div>
            <FleetAgentCards agents={owner.agents} now={now} owner={owner} />
          </div>
        );
      })}
    </section>
  );
}
