export type AgentManagementDelegationCapabilities = {
  agent_config: boolean;
  global_config: boolean;
  teams: boolean;
  agent_creation: boolean;
  env_vars: boolean;
};

export type CapabilitySubject =
  | { scope: "none" }
  | { scope: "all" }
  | { scope: "set"; grantees: string[] };

export type AgentManagementDelegationSubjects = {
  agent_config: CapabilitySubject;
  global_config: CapabilitySubject;
  teams: CapabilitySubject;
  agent_creation: CapabilitySubject;
  env_vars: CapabilitySubject;
};

export type AgentManagementDelegationGrantV1 = {
  version?: undefined;
  granteePubkey: string;
  auditChannelId: string;
  capabilities: AgentManagementDelegationCapabilities;
  signedEvent?: string | null;
};

export type AgentManagementDelegationGrantV2 = {
  version: 2;
  auditChannelId: string;
  capabilities: AgentManagementDelegationSubjects;
  signedEvent?: string | null;
};

export type AgentEditScope = {
  targetPubkeys: string[];
  allowedFields: ("displayName" | "systemPrompt")[];
  allowSelfEdit: boolean;
};

export type AgentManagementDelegationGrantV4 = Omit<
  AgentManagementDelegationGrantV1,
  "version"
> & {
  version: 4;
  relayScope: string;
  agentEdit: AgentEditScope;
};

export type AgentManagementDelegationGrant =
  | AgentManagementDelegationGrantV1
  | AgentManagementDelegationGrantV2
  | AgentManagementDelegationGrantV4;
