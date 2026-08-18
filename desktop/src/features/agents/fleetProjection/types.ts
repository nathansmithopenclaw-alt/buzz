/**
 * Public, relay-derived fleet data. These types are intentionally separate
 * from ManagedAgent and cannot represent runnable configuration: there is no
 * private-key material, auth material, environment, command, or backend data.
 */

export type FleetProjectionRespondTo = "owner-only" | "allowlist" | "anyone";

export type FleetProjectionLifecycleState = "running" | "stopped" | "error";

export type FleetProjectionAgent = {
  ownerPubkey: string;
  agentPubkey: string;
  name: string;
  personaId: string | null;
  personaDisplayName: string | null;
  personaAvatarUrl: string | null;
  personaRuntime: string | null;
  personaModel: string | null;
  respondTo: FleetProjectionRespondTo;
  respondToAllowlist: readonly string[];
};

export type FleetProjectionPersona = {
  ownerPubkey: string;
  personaId: string;
  displayName: string;
  avatarUrl: string | null;
  runtime: string | null;
  model: string | null;
};

export type FleetProjectionAgentStatus = {
  pubkey: string;
  state: FleetProjectionLifecycleState;
  since: number;
};

export type FleetProjectionHostStatus = {
  seq: number;
  timestamp: number;
  agents: readonly FleetProjectionAgentStatus[];
};

export type FleetProjectionHost = {
  ownerPubkey: string;
  hostId: string;
  name: string | null;
  platform: "macos" | "linux" | "windows" | null;
  declaredAt: number | null;
  status: FleetProjectionHostStatus | null;
};

export type FleetProjectionOwner = {
  ownerPubkey: string;
  agents: readonly FleetProjectionAgent[];
  personas: readonly FleetProjectionPersona[];
  hosts: readonly FleetProjectionHost[];
};

export type FleetProjectionSnapshot = {
  owners: readonly FleetProjectionOwner[];
  loading: boolean;
  errorMessage: string | null;
};
