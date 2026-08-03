export type AgentModelsResponse = {
  agentName: string;
  agentVersion: string;
  models: AgentModelInfo[];
  agentDefaultModel: string | null;
  selectedModel: string | null;
  supportsSwitching: boolean;
  effortConfigId?: string | null;
  effortOptions?: Array<{ id: string; name: string | null }>;
  fastModeConfigId?: string | null;
  fastModeEnabled?: boolean | null;
  fastModeDescription?: string | null;
};

export type AgentModelInfo = {
  id: string;
  name: string | null;
  description: string | null;
};
