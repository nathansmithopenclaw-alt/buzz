export const AGENT_TEAM_MANAGEMENT_REQUEST =
  "agent_team_management_request" as const;

type TeamCreateRequest = {
  type: typeof AGENT_TEAM_MANAGEMENT_REQUEST;
  action: "create";
  requestId: string;
  intent: "apply";
  request: {
    channelId: string;
    name: string;
    description?: string;
    instructions?: string;
    members: string[];
  };
};

type TeamUpdateRequest = {
  type: typeof AGENT_TEAM_MANAGEMENT_REQUEST;
  action: "update";
  requestId: string;
  intent: "apply";
  request: {
    channelId: string;
    team: string;
    name?: string;
    description?: string;
    instructions?: string;
    members?: string[];
  };
};

type TeamAddToChannelRequest = {
  type: typeof AGENT_TEAM_MANAGEMENT_REQUEST;
  action: "add_to_channel";
  requestId: string;
  intent: "apply";
  request: { channelId: string; team: string };
};

export type AgentTeamManagementRequest =
  | TeamCreateRequest
  | TeamUpdateRequest
  | TeamAddToChannelRequest;

function isText(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isOptionalText(value: unknown): value is string | undefined {
  return value === undefined || typeof value === "string";
}

function isMembers(value: unknown): value is string[] {
  return Array.isArray(value) && value.length <= 100 && value.every(isText);
}

function onlyKeys(value: Record<string, unknown>, keys: readonly string[]) {
  return Object.keys(value).every((key) => keys.includes(key));
}

export function parseAgentTeamManagementRequest(
  value: unknown,
): AgentTeamManagementRequest | null {
  if (typeof value !== "object" || value === null) return null;
  const payload = value as Record<string, unknown>;
  if (
    payload.type !== AGENT_TEAM_MANAGEMENT_REQUEST ||
    payload.intent !== "apply" ||
    !isText(payload.requestId) ||
    !onlyKeys(payload, ["type", "action", "requestId", "intent", "request"]) ||
    typeof payload.request !== "object" ||
    payload.request === null
  ) {
    return null;
  }
  const request = payload.request as Record<string, unknown>;
  if (!isText(request.channelId)) return null;
  if (payload.action === "create") {
    if (
      !isText(request.name) ||
      !isOptionalText(request.description) ||
      !isOptionalText(request.instructions) ||
      !isMembers(request.members) ||
      !onlyKeys(request, [
        "channelId",
        "name",
        "description",
        "instructions",
        "members",
      ])
    ) {
      return null;
    }
    return {
      type: AGENT_TEAM_MANAGEMENT_REQUEST,
      action: "create",
      requestId: payload.requestId,
      intent: "apply",
      request: {
        channelId: request.channelId,
        name: request.name,
        ...(typeof request.description === "string"
          ? { description: request.description }
          : {}),
        ...(typeof request.instructions === "string"
          ? { instructions: request.instructions }
          : {}),
        members: request.members,
      },
    };
  }
  if (payload.action === "update") {
    if (
      !isText(request.team) ||
      !isOptionalText(request.name) ||
      !isOptionalText(request.description) ||
      !isOptionalText(request.instructions) ||
      (request.members !== undefined && !isMembers(request.members)) ||
      !onlyKeys(request, [
        "channelId",
        "team",
        "name",
        "description",
        "instructions",
        "members",
      ])
    ) {
      return null;
    }
    return {
      type: AGENT_TEAM_MANAGEMENT_REQUEST,
      action: "update",
      requestId: payload.requestId,
      intent: "apply",
      request: request as TeamUpdateRequest["request"],
    };
  }
  if (
    payload.action === "add_to_channel" &&
    isText(request.team) &&
    onlyKeys(request, ["channelId", "team"])
  ) {
    return {
      type: AGENT_TEAM_MANAGEMENT_REQUEST,
      action: "add_to_channel",
      requestId: payload.requestId,
      intent: "apply",
      request: { channelId: request.channelId, team: request.team },
    };
  }
  return null;
}
