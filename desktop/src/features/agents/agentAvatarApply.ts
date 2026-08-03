export const AGENT_AVATAR_APPLY_REQUEST = "agent_avatar_apply_request" as const;

export type AgentAvatarApplyRequest = {
  type: typeof AGENT_AVATAR_APPLY_REQUEST;
  requestId: string;
  authorizationEvent: string;
};

function isText(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

/** Parse only the relay envelope. Rust verifies the embedded owner signature. */
export function parseAgentAvatarApplyRequest(
  value: unknown,
): AgentAvatarApplyRequest | null {
  if (typeof value !== "object" || value === null) return null;
  const payload = value as Record<string, unknown>;
  if (
    payload.type !== AGENT_AVATAR_APPLY_REQUEST ||
    !isText(payload.requestId) ||
    !isText(payload.authorizationEvent) ||
    Object.keys(payload).some(
      (key) => !["type", "requestId", "authorizationEvent"].includes(key),
    )
  ) {
    return null;
  }
  return {
    type: AGENT_AVATAR_APPLY_REQUEST,
    requestId: payload.requestId,
    authorizationEvent: payload.authorizationEvent,
  };
}
