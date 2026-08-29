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
  grants: Readonly<Record<string, AgentManagementDelegationGrant>> = {},
): AgentManagementDelegationSubjects {
  let subjects = emptySubjects();
  const mergeSubject = (
    current: CapabilitySubject,
    incoming: CapabilitySubject,
  ): CapabilitySubject => {
    if (current.scope === "all" || incoming.scope === "all") {
      return allSubject();
    }
    const selected = new Set<string>();
    if (current.scope === "set") {
      for (const pubkey of current.grantees) selected.add(pubkey.toLowerCase());
    }
    if (incoming.scope === "set") {
      for (const pubkey of incoming.grantees)
        selected.add(pubkey.toLowerCase());
    }
    return selected.size === 0
      ? noneSubject()
      : { scope: "set", grantees: [...selected].sort() };
  };
  const bit = (enabled: boolean): CapabilitySubject =>
    enabled ? { scope: "set", grantees: [] } : noneSubject();
  const mergeGrant = (nextGrant: AgentManagementDelegationGrant) => {
    let incoming: AgentManagementDelegationSubjects;
    if (isV2Grant(nextGrant)) {
      incoming = nextGrant.capabilities;
    } else if (
      "granteePubkey" in nextGrant &&
      typeof nextGrant.granteePubkey === "string"
    ) {
      const pubkey = nextGrant.granteePubkey.toLowerCase();
      const grantedBit = (enabled: boolean): CapabilitySubject => {
        const subject = bit(enabled);
        return subject.scope === "set"
          ? { scope: "set", grantees: [pubkey] }
          : subject;
      };
      incoming = {
        agent_config: grantedBit(nextGrant.capabilities.agent_config),
        agent_creation: grantedBit(nextGrant.capabilities.agent_creation),
        env_vars: grantedBit(nextGrant.capabilities.env_vars),
        global_config: grantedBit(nextGrant.capabilities.global_config),
        teams: grantedBit(nextGrant.capabilities.teams),
      };
    } else {
      return;
    }
    subjects = {
      agent_config: mergeSubject(subjects.agent_config, incoming.agent_config),
      agent_creation: mergeSubject(
        subjects.agent_creation,
        incoming.agent_creation,
      ),
      env_vars: mergeSubject(subjects.env_vars, incoming.env_vars),
      global_config: mergeSubject(
        subjects.global_config,
        incoming.global_config,
      ),
      teams: mergeSubject(subjects.teams, incoming.teams),
    };
  };

  if (grant) mergeGrant(grant);
  for (const nextGrant of Object.values(grants)) mergeGrant(nextGrant);
  return subjects;
}

export function grantedPubkeys(
  subjects: AgentManagementDelegationSubjects,
  roster: readonly string[],
): string[] {
  const selected = new Set<string>();
  for (const [capability] of DELEGATION_CAPABILITIES) {
    const subject = subjects[capability];
    if (subject.scope === "all") {
      for (const pubkey of roster) selected.add(pubkey.toLowerCase());
    } else if (subject.scope === "set") {
      for (const pubkey of subject.grantees) selected.add(pubkey.toLowerCase());
    }
  }
  return [...selected].sort();
}

export function removeGranteeFromSubjects(
  subjects: AgentManagementDelegationSubjects,
  pubkey: string,
  roster: readonly string[],
): AgentManagementDelegationSubjects {
  const hex = pubkey.toLowerCase();
  return Object.fromEntries(
    DELEGATION_CAPABILITIES.map(([capability]) => {
      const subject = subjects[capability];
      const selected =
        subject.scope === "all"
          ? roster.map((item) => item.toLowerCase())
          : subject.scope === "set"
            ? subject.grantees.map((item) => item.toLowerCase())
            : [];
      const next = [...new Set(selected.filter((item) => item !== hex))].sort();
      return [
        capability,
        next.length === 0
          ? noneSubject()
          : { scope: "set" as const, grantees: next },
      ];
    }),
  ) as AgentManagementDelegationSubjects;
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
