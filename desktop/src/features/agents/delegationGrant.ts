import type {
  AgentManagementDelegationGrant,
  AgentManagementDelegationSubjects,
  CapabilitySubject,
} from "@/shared/api/types";

export const DELEGATION_CAPABILITIES = [
  ["agent_config", "Update existing agents"],
  ["agent_creation", "Create agents"],
  ["env_vars", "Set agent environment variables"],
  ["global_config", "Update global agent defaults"],
  ["teams", "Create and update teams"],
] as const;

export type DelegationCapability = (typeof DELEGATION_CAPABILITIES)[number][0];

export function noneSubject(): CapabilitySubject {
  return { scope: "none" };
}

export function allSubject(): CapabilitySubject {
  return { scope: "all" };
}

export function emptySubjects(): AgentManagementDelegationSubjects {
  return {
    agent_config: noneSubject(),
    agent_creation: noneSubject(),
    env_vars: noneSubject(),
    global_config: noneSubject(),
    teams: noneSubject(),
  };
}

export function allSubjects(): AgentManagementDelegationSubjects {
  return {
    agent_config: allSubject(),
    agent_creation: allSubject(),
    env_vars: allSubject(),
    global_config: allSubject(),
    teams: allSubject(),
  };
}

export function isV2Grant(
  grant: AgentManagementDelegationGrant,
): grant is Extract<AgentManagementDelegationGrant, { version: 2 }> {
  return grant.version === 2;
}

export function grantSubjects(
  grant: AgentManagementDelegationGrant | null,
): AgentManagementDelegationSubjects {
  if (!grant) return emptySubjects();
  if (isV2Grant(grant)) return grant.capabilities;
  if (!("granteePubkey" in grant) || typeof grant.granteePubkey !== "string") {
    return emptySubjects();
  }
  const bit = (enabled: boolean): CapabilitySubject =>
    enabled
      ? { scope: "set", grantees: [grant.granteePubkey.toLowerCase()] }
      : noneSubject();
  return {
    agent_config: bit(grant.capabilities.agent_config),
    agent_creation: bit(grant.capabilities.agent_creation),
    env_vars: bit(grant.capabilities.env_vars),
    global_config: bit(grant.capabilities.global_config),
    teams: bit(grant.capabilities.teams),
  };
}

export function subjectsAreEmpty(
  subjects: AgentManagementDelegationSubjects,
): boolean {
  return DELEGATION_CAPABILITIES.every(
    ([capability]) => subjects[capability].scope === "none",
  );
}

export function subjectIsOn(subject: CapabilitySubject): boolean {
  return subject.scope !== "none";
}

export function subjectSummary(subject: CapabilitySubject): string {
  if (subject.scope === "none") return "Off";
  if (subject.scope === "all") return "All agents";
  const count = subject.grantees.length;
  return count === 1 ? "1 agent" : `${count} agents`;
}

export function agentIsGranted(
  subject: CapabilitySubject,
  pubkey: string,
): boolean {
  const hex = pubkey.toLowerCase();
  if (subject.scope === "all") return true;
  if (subject.scope === "none") return false;
  return subject.grantees.some((grantee) => grantee.toLowerCase() === hex);
}

/**
 * Toggle one agent on a subject. Unchecking while `all` is on leaves `all`
 * and writes `set` of the current roster minus that name.
 */
export function toggleGrantedAgent(
  subject: CapabilitySubject,
  pubkey: string,
  roster: readonly string[],
): { subject: CapabilitySubject; leftAll: boolean } {
  const hex = pubkey.toLowerCase();
  const canonicalRoster = [
    ...new Set(roster.map((item) => item.toLowerCase())),
  ];

  if (subject.scope === "all") {
    const next = canonicalRoster.filter((item) => item !== hex);
    return {
      subject:
        next.length === 0 ? noneSubject() : { scope: "set", grantees: next },
      leftAll: true,
    };
  }

  if (subject.scope === "none") {
    return { subject: { scope: "set", grantees: [hex] }, leftAll: false };
  }

  const has = subject.grantees.some((grantee) => grantee.toLowerCase() === hex);
  const next = has
    ? subject.grantees.filter((grantee) => grantee.toLowerCase() !== hex)
    : [...subject.grantees.map((grantee) => grantee.toLowerCase()), hex];
  if (next.length === 0) return { subject: noneSubject(), leftAll: false };
  return { subject: { scope: "set", grantees: next }, leftAll: false };
}

export const LEFT_ALL_NOTICE =
  "Future agents will no longer be covered until All agents is checked again.";
