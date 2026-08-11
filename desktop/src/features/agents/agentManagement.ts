import type {
  AgentPersona,
  CreatePersonaInput,
  RespondToMode,
  UpdatePersonaInput,
} from "@/shared/api/types";

export const AGENT_MANAGEMENT_REQUEST = "agent_management_request" as const;

export type AgentManagementCreateRequest = {
  type: typeof AGENT_MANAGEMENT_REQUEST;
  action: "create";
  requestId: string;
  intent: "draft" | "apply";
  request: {
    channelId: string;
    displayName: string;
    systemPrompt: string;
    runtime?: string;
    provider?: string;
    model?: string;
    respondTo?: RespondToMode;
    parallelism?: number;
    envVars?: Record<string, string>;
  };
};

export type AgentManagementUpdateRequest = {
  type: typeof AGENT_MANAGEMENT_REQUEST;
  action: "update";
  requestId: string;
  intent: "draft" | "apply";
  request: {
    channelId: string;
    agentName: string;
    displayName?: string;
    systemPrompt?: string;
    runtime?: string;
    provider?: string;
    model?: string;
    effort?: string;
    respondTo?: RespondToMode;
    respondToAllowlist?: string[];
    parallelism?: number;
    namePool?: string[];
    envVars?: Record<string, string>;
  };
};

export type AgentManagementGlobalConfigRequest = {
  type: typeof AGENT_MANAGEMENT_REQUEST;
  action: "global_config";
  requestId: string;
  intent: "draft" | "apply";
  request: {
    channelId: string;
    provider?: string;
    model?: string;
    preferredRuntime?: string;
    effort?: string;
    envVars?: Record<string, string>;
  };
};

export type AgentManagementRequest =
  | AgentManagementCreateRequest
  | AgentManagementUpdateRequest
  | AgentManagementGlobalConfigRequest;

const EFFORT_VALUES = new Set([
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
]);

function isText(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isOptionalText(value: unknown): value is string | undefined {
  return value === undefined || isText(value);
}

function isRespondTo(value: unknown): value is RespondToMode | undefined {
  return (
    value === undefined ||
    value === "owner-only" ||
    value === "allowlist" ||
    value === "anyone"
  );
}

function isParallelism(value: unknown): value is number | undefined {
  return (
    value === undefined ||
    (Number.isInteger(value) && Number(value) >= 1 && Number(value) <= 32)
  );
}

function isNamePool(value: unknown): value is string[] | undefined {
  return (
    value === undefined ||
    (Array.isArray(value) && value.length <= 100 && value.every(isText))
  );
}

function isAllowlist(value: unknown): value is string[] | undefined {
  return isNamePool(value);
}

function isEffort(value: unknown): value is string | undefined {
  return (
    value === undefined ||
    (typeof value === "string" && EFFORT_VALUES.has(value))
  );
}

function isEnvVars(
  value: unknown,
): value is Record<string, string> | undefined {
  return (
    value === undefined ||
    (typeof value === "object" &&
      value !== null &&
      !Array.isArray(value) &&
      Object.entries(value).every(
        ([key, entry]) => key.length > 0 && typeof entry === "string",
      ))
  );
}

function hasOnlyKeys(
  value: Record<string, unknown>,
  allowed: readonly string[],
) {
  return Object.keys(value).every((key) => allowed.includes(key));
}

/** Parses only the deliberately narrow no-secret agent-management request contract. */
export function parseAgentManagementRequest(
  value: unknown,
): AgentManagementRequest | null {
  if (typeof value !== "object" || value === null) return null;
  const payload = value as Record<string, unknown>;
  if (
    payload.type !== AGENT_MANAGEMENT_REQUEST ||
    !isText(payload.requestId) ||
    (payload.intent !== undefined &&
      payload.intent !== "draft" &&
      payload.intent !== "apply") ||
    !hasOnlyKeys(payload, [
      "type",
      "action",
      "requestId",
      "intent",
      "request",
    ]) ||
    (payload.action !== "create" &&
      payload.action !== "update" &&
      payload.action !== "global_config") ||
    typeof payload.request !== "object" ||
    payload.request === null
  ) {
    return null;
  }
  const request = payload.request as Record<string, unknown>;

  if (payload.action === "global_config") {
    if (
      !isOptionalText(request.provider) ||
      !isOptionalText(request.model) ||
      !isOptionalText(request.preferredRuntime) ||
      !isEffort(request.effort) ||
      !isEnvVars(request.envVars) ||
      !hasOnlyKeys(request, [
        "channelId",
        "provider",
        "model",
        "preferredRuntime",
        "effort",
        "envVars",
      ]) ||
      !isText(request.channelId)
    ) {
      return null;
    }
    const changes = {
      ...(isText(request.provider) ? { provider: request.provider } : {}),
      ...(isText(request.model) ? { model: request.model } : {}),
      ...(isText(request.preferredRuntime)
        ? { preferredRuntime: request.preferredRuntime }
        : {}),
      ...(isEffort(request.effort) && request.effort !== undefined
        ? { effort: request.effort }
        : {}),
      ...(request.envVars && Object.keys(request.envVars).length > 0
        ? { envVars: request.envVars }
        : {}),
    };
    if (Object.keys(changes).length === 0) return null;
    return {
      type: AGENT_MANAGEMENT_REQUEST,
      action: "global_config",
      requestId: payload.requestId,
      intent: payload.intent === "apply" ? "apply" : "draft",
      request: { channelId: request.channelId, ...changes },
    };
  }

  if (payload.action === "create") {
    if (
      !isOptionalText(request.runtime) ||
      !isOptionalText(request.provider) ||
      !isOptionalText(request.model) ||
      !isRespondTo(request.respondTo) ||
      request.respondTo === "allowlist" ||
      !isParallelism(request.parallelism) ||
      !isEnvVars(request.envVars) ||
      !hasOnlyKeys(request, [
        "channelId",
        "displayName",
        "systemPrompt",
        "runtime",
        "provider",
        "model",
        "respondTo",
        "parallelism",
        "envVars",
      ])
    ) {
      return null;
    }
    if (
      !isText(request.channelId) ||
      !isText(request.displayName) ||
      !isText(request.systemPrompt)
    ) {
      return null;
    }
    return {
      type: AGENT_MANAGEMENT_REQUEST,
      action: "create",
      requestId: payload.requestId,
      intent: payload.intent === "apply" ? "apply" : "draft",
      request: {
        channelId: request.channelId,
        displayName: request.displayName,
        systemPrompt: request.systemPrompt,
        ...(isText(request.runtime) ? { runtime: request.runtime } : {}),
        ...(isText(request.provider) ? { provider: request.provider } : {}),
        ...(isText(request.model) ? { model: request.model } : {}),
        ...(request.respondTo ? { respondTo: request.respondTo } : {}),
        ...(request.parallelism ? { parallelism: request.parallelism } : {}),
        ...(request.envVars ? { envVars: request.envVars } : {}),
      },
    };
  }

  if (
    !isOptionalText(request.displayName) ||
    !isOptionalText(request.systemPrompt) ||
    !isOptionalText(request.runtime) ||
    !isOptionalText(request.provider) ||
    !isOptionalText(request.model) ||
    !isEffort(request.effort) ||
    !isRespondTo(request.respondTo) ||
    !isAllowlist(request.respondToAllowlist) ||
    !isParallelism(request.parallelism) ||
    !isNamePool(request.namePool) ||
    !isEnvVars(request.envVars) ||
    !hasOnlyKeys(request, [
      "channelId",
      "agentName",
      "displayName",
      "systemPrompt",
      "runtime",
      "provider",
      "model",
      "effort",
      "respondTo",
      "respondToAllowlist",
      "parallelism",
      "namePool",
      "envVars",
    ]) ||
    !isText(request.channelId) ||
    !isText(request.agentName)
  ) {
    return null;
  }
  if (
    request.respondTo === "allowlist" &&
    (!request.respondToAllowlist || request.respondToAllowlist.length === 0)
  ) {
    return null;
  }
  const changes = {
    ...(isText(request.displayName)
      ? { displayName: request.displayName }
      : {}),
    ...(isText(request.systemPrompt)
      ? { systemPrompt: request.systemPrompt }
      : {}),
    ...(isText(request.runtime) ? { runtime: request.runtime } : {}),
    ...(isText(request.provider) ? { provider: request.provider } : {}),
    ...(isText(request.model) ? { model: request.model } : {}),
    ...(isEffort(request.effort) && request.effort !== undefined
      ? { effort: request.effort }
      : {}),
    ...(request.respondTo ? { respondTo: request.respondTo } : {}),
    ...(request.respondToAllowlist
      ? { respondToAllowlist: request.respondToAllowlist }
      : {}),
    ...(request.parallelism ? { parallelism: request.parallelism } : {}),
    ...(request.namePool ? { namePool: request.namePool } : {}),
    ...(request.envVars ? { envVars: request.envVars } : {}),
  };
  if (Object.keys(changes).length === 0) return null;
  return {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: payload.requestId,
    intent: payload.intent === "apply" ? "apply" : "draft",
    request: {
      channelId: request.channelId,
      agentName: request.agentName,
      ...changes,
    },
  };
}

export function requestTargetsEditablePersona(
  persona: AgentPersona | undefined,
): persona is AgentPersona {
  return Boolean(persona && !persona.sourceTeam);
}

export function createInputFromRequest(
  request: Extract<AgentManagementRequest, { action: "create" }>,
): CreatePersonaInput {
  return {
    displayName: request.request.displayName,
    systemPrompt: request.request.systemPrompt,
    ...(request.request.runtime ? { runtime: request.request.runtime } : {}),
    ...(request.request.provider ? { provider: request.request.provider } : {}),
    ...(request.request.model ? { model: request.request.model } : {}),
    ...(request.request.envVars ? { envVars: request.request.envVars } : {}),
    ...(request.request.respondTo || request.request.parallelism
      ? {
          behavior: {
            respondTo: request.request.respondTo,
            parallelism: request.request.parallelism,
          },
        }
      : {}),
  };
}

export function updateInputFromRequest(
  request: Extract<AgentManagementRequest, { action: "update" }>,
  current: UpdatePersonaInput,
  effortEnvVar?: string | null,
): UpdatePersonaInput {
  const changes = request.request;
  if (changes.effort && !effortEnvVar) {
    throw new Error(
      "The selected runtime does not support effort configuration.",
    );
  }
  const behaviorChanged =
    changes.respondTo !== undefined ||
    changes.respondToAllowlist !== undefined ||
    changes.parallelism !== undefined;
  return {
    ...current,
    displayName: changes.displayName ?? current.displayName,
    systemPrompt: changes.systemPrompt ?? current.systemPrompt,
    runtime: changes.runtime ?? current.runtime,
    provider: changes.provider ?? current.provider,
    model: changes.model ?? current.model,
    namePool: changes.namePool ?? current.namePool,
    ...(changes.envVars || changes.effort
      ? {
          envVars: {
            ...(current.envVars ?? {}),
            ...changes.envVars,
            ...(changes.effort && effortEnvVar
              ? { [effortEnvVar]: changes.effort }
              : {}),
          },
        }
      : {}),
    ...(behaviorChanged
      ? {
          behavior: {
            respondTo: changes.respondTo ?? current.behavior?.respondTo,
            respondToAllowlist:
              changes.respondTo === "owner-only" ||
              changes.respondTo === "anyone"
                ? []
                : (changes.respondToAllowlist ??
                  current.behavior?.respondToAllowlist),
            parallelism: changes.parallelism ?? current.behavior?.parallelism,
          },
        }
      : {}),
  };
}

export function globalConfigPatchFromRequest(
  request: Extract<AgentManagementRequest, { action: "global_config" }>,
  effortEnvVar?: string | null,
) {
  const changes = request.request;
  if (changes.effort && !effortEnvVar) {
    throw new Error(
      "The preferred runtime does not support effort configuration.",
    );
  }
  return {
    ...(changes.provider ? { provider: changes.provider } : {}),
    ...(changes.model ? { model: changes.model } : {}),
    ...(changes.preferredRuntime
      ? { preferredRuntime: changes.preferredRuntime }
      : {}),
    envVars: {
      ...changes.envVars,
      ...(changes.effort && effortEnvVar
        ? { [effortEnvVar]: changes.effort }
        : {}),
    },
  };
}

export function globalConfigAuditTarget(result: unknown): string {
  if (typeof result !== "object" || result === null) {
    return "saved global agent defaults";
  }
  const restarted = (result as { restartedCount?: unknown }).restartedCount;
  const failed = (result as { failedRestartCount?: unknown })
    .failedRestartCount;
  if (typeof restarted !== "number" || typeof failed !== "number") {
    return "saved global agent defaults";
  }
  return restarted === 0 && failed === 0
    ? "saved global agent defaults; no running agent was respawned"
    : `saved global agent defaults; restarted ${restarted} affected running agent(s), ${failed} restart failure(s)`;
}

export function globalConfigReviewRows(
  request: AgentManagementGlobalConfigRequest | null,
): Array<[string, string]> {
  const changes = request?.request;
  if (!changes) return [];
  return [
    changes.provider ? ["Provider", changes.provider] : null,
    changes.model ? ["Model", changes.model] : null,
    changes.preferredRuntime
      ? ["Preferred runtime", changes.preferredRuntime]
      : null,
    changes.effort ? ["Default effort", changes.effort] : null,
    changes.envVars
      ? ["Environment keys", Object.keys(changes.envVars).sort().join(", ")]
      : null,
  ].filter((row): row is [string, string] => row !== null);
}
