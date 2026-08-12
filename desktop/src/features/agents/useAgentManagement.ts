import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  createInputFromRequest,
  globalConfigAuditTarget,
  parseAgentManagementRequest,
  requestTargetsEditablePersona,
  updateInputFromRequest,
  type AgentManagementRequest,
} from "./agentManagement";
import { subscribeAgentManagementRequests } from "./observerRelayStore";
import {
  managedAgentsQueryKey,
  personasQueryKey,
  useAcpRuntimesQuery,
  useCreateManagedAgentMutation,
  useCreatePersonaMutation,
  useManagedAgentsQuery,
  usePersonasQuery,
  useUpdatePersonaMutation,
} from "./hooks";
import {
  availableRuntimesForStart,
  buildInstanceInputForDefinition,
  type BackendIntent,
} from "./lib/instanceInputForDefinition";
import { getDefaultPersonaRuntime } from "./lib/resolvePersonaRuntime";
import { useCreatedAgentChannelAttachment } from "./useCreatedAgentChannelAttachment";
import {
  classifyAgentManagementOrigin,
  dequeueAgentManagementRequest,
  enqueueAgentManagementRequest,
  type QueuedAgentManagementRequest,
} from "./agentManagementBuffer";
import { useGlobalAgentConfig } from "./useGlobalAgentConfig";
import { globalAgentConfigQueryKey } from "./useGlobalAgentConfig";
import {
  beginOwnerReviewedAgentManagementRequest,
  completeAgentManagementRequest,
  dismissAgentManagementRequest,
  getManagedAgentNsec,
  publishAgentCreateSecretResponse,
  publishAgentManagementAuditReceipt,
  publishAgentManagementResult,
  restoreAgentManagementRequests,
} from "@/shared/api/tauriManagedAgents";
import { useChannelsQuery } from "@/features/channels/hooks";
import { resolveManagedAgentAvatarUrl } from "./ui/managedAgentAvatar";
import type { AgentCreateIntent } from "./ui/agentCreateIntent";
import { editPersonaDialogState } from "./ui/personaDialogState";
import type {
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import {
  capabilityForAgentManagementRequest,
  registerAndRouteAgentManagementRequest,
} from "./agentManagementRegistration";
import {
  applyDelegatedGlobalConfig,
  completeDelegatedGlobalConfigFailure,
  completeDelegatedGlobalConfigSuccess,
} from "./delegatedGlobalConfig";
import {
  applyDelegatedPersonaEnvVarsRequest,
  delegatedEnvAuditTarget,
} from "./delegatedPersonaEnvVars";
import {
  completeDelegatedAgentUpdateFailure,
  completeDelegatedAgentUpdateSuccess,
} from "./delegatedAgentUpdateCompletion";

function requesterNameFor(
  agents: readonly { pubkey: string; name: string }[] | undefined,
  sourcePubkey: string,
): string {
  return (
    agents?.find(
      (agent) => agent.pubkey.toLowerCase() === sourcePubkey.toLowerCase(),
    )?.name ?? "delegated agent"
  );
}

export function useAgentManagement() {
  const queryClient = useQueryClient();
  const personasQuery = usePersonasQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const channelsQuery = useChannelsQuery();
  const runtimesQuery = useAcpRuntimesQuery({ enabled: true });
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const createAgentMutation = useCreateManagedAgentMutation();
  const { globalConfig, isLoading: globalConfigLoading } =
    useGlobalAgentConfig();
  const [request, setRequest] = React.useState<AgentManagementRequest | null>(
    null,
  );
  const [error, setError] = React.useState<string | null>(null);
  const [globalConfigPending, setGlobalConfigPending] = React.useState(false);
  const createdAgentAttachment = useCreatedAgentChannelAttachment();
  const seenRequestIds = React.useRef(new Set<string>());
  const pendingRequestId = React.useRef<string | null>(null);
  const sourceAgentPubkey = React.useRef<string | null>(null);
  const acceptedRequestsRef = React.useRef<QueuedAgentManagementRequest[]>([]);
  const autoApprovingRequestId = React.useRef<string | null>(null);
  const [failedAutoApprovalRequestId, setFailedAutoApprovalRequestId] =
    React.useState<string | null>(null);
  const managedAgentsRef = React.useRef(managedAgentsQuery.data);
  const channelsRef = React.useRef(channelsQuery.data);
  const bufferedRequestsRef = React.useRef<
    Array<{ agentPubkey: string; request: AgentManagementRequest }>
  >([]);
  const registrationQueueRef = React.useRef(Promise.resolve());
  const delegatedApplyIdsRef = React.useRef(new Set<string>());

  function updateCompletionContext(
    requestId: string,
    sourcePubkey: string,
    updateRequest: Extract<AgentManagementRequest, { action: "update" }>,
  ) {
    return {
      requestId,
      sourcePubkey,
      requesterName: requesterNameFor(managedAgentsRef.current, sourcePubkey),
      request: updateRequest,
    };
  }

  const presentNextRequest = React.useEffectEvent(() => {
    if (pendingRequestId.current !== null) return;
    const next = dequeueAgentManagementRequest(acceptedRequestsRef.current);
    if (!next) return;
    pendingRequestId.current = next.request.requestId;
    sourceAgentPubkey.current = next.agentPubkey;
    setFailedAutoApprovalRequestId(null);
    setError(null);
    setRequest(next.request);
  });

  const acceptOwnedRequest = React.useEffectEvent(
    (agentPubkey: string, next: AgentManagementRequest) => {
      if (
        classifyAgentManagementOrigin(
          managedAgentsRef.current,
          channelsRef.current,
          agentPubkey,
          next.request.channelId,
        ) !== "accept"
      ) {
        return;
      }
      registrationQueueRef.current = registrationQueueRef.current
        .then(async () => {
          const route = await registerAndRouteAgentManagementRequest(
            agentPubkey,
            next,
          );
          if (route.kind === "delegated_apply") {
            delegatedApplyIdsRef.current.add(next.requestId);
          } else if (
            route.kind === "terminal_duplicate" &&
            (route.registered.disposition === "duplicate_failed" ||
              route.registered.disposition === "rejected") &&
            route.registered.result
          ) {
            const registered = route.registered;
            const result = registered.result as Record<string, unknown>;
            await publishAgentManagementAuditReceipt({
              requestId: next.requestId,
              sourcePubkey: agentPubkey,
              requesterName: requesterNameFor(
                managedAgentsRef.current,
                agentPubkey,
              ),
              capability: capabilityForAgentManagementRequest(next),
              target: `agent ${next.action}: ${String(result.error ?? "failed")}`,
              succeeded: false,
            }).catch(() => undefined);
            await publishAgentManagementResult({
              requestId: next.requestId,
              sourcePubkey: agentPubkey,
              result: registered.result,
            });
            return;
          } else if (
            route.kind === "terminal_duplicate" &&
            route.registered.disposition === "duplicate_applied" &&
            next.action !== "create" &&
            route.registered.result
          ) {
            await publishAgentManagementAuditReceipt({
              requestId: next.requestId,
              sourcePubkey: agentPubkey,
              requesterName: requesterNameFor(
                managedAgentsRef.current,
                agentPubkey,
              ),
              capability: capabilityForAgentManagementRequest(next),
              target:
                next.action === "update"
                  ? capabilityForAgentManagementRequest(next) === "env_vars"
                    ? delegatedEnvAuditTarget(
                        next.request.agentName,
                        route.registered.result as {
                          restartedCount: number;
                          startedCount: number;
                          failedStartCount: number;
                        },
                      )
                    : `saved config for agent ${next.request.agentName}; instruction and runtime changes take effect on next spawn`
                  : globalConfigAuditTarget(route.registered.result),
              succeeded: true,
            });
            await publishAgentManagementResult({
              requestId: next.requestId,
              sourcePubkey: agentPubkey,
              result: route.registered.result,
            });
            return;
          } else if (
            route.kind === "terminal_duplicate" &&
            route.registered.disposition === "duplicate_applied" &&
            next.action === "create" &&
            typeof route.registered.result === "object" &&
            route.registered.result !== null &&
            typeof (route.registered.result as { pubkey?: unknown }).pubkey ===
              "string"
          ) {
            const createdPubkey = (
              route.registered.result as { pubkey: string }
            ).pubkey;
            const privateKeyNsec = await getManagedAgentNsec(createdPubkey);
            const createdAgent = managedAgentsRef.current?.find(
              (agent) =>
                agent.pubkey.toLowerCase() === createdPubkey.toLowerCase(),
            );
            await publishAgentManagementAuditReceipt({
              requestId: next.requestId,
              sourcePubkey: agentPubkey,
              requesterName: requesterNameFor(
                managedAgentsRef.current,
                agentPubkey,
              ),
              capability: "agent_creation",
              target: `created agent ${createdAgent?.name ?? createdPubkey} (${createdPubkey})`,
              succeeded: true,
            });
            await publishAgentCreateSecretResponse({
              requestId: next.requestId,
              sourcePubkey: agentPubkey,
              agentPubkey: createdPubkey,
              privateKeyNsec,
            });
            return;
          } else if (route.kind !== "draft") {
            return;
          }
          if (
            enqueueAgentManagementRequest(
              acceptedRequestsRef.current,
              seenRequestIds.current,
              { agentPubkey, request: next },
            )
          ) {
            presentNextRequest();
          }
        })
        .catch((cause) => {
          setError(
            cause instanceof Error
              ? cause.message
              : "Could not register agent-management request.",
          );
        });
    },
  );

  React.useEffect(() => {
    let cancelled = false;
    void restoreAgentManagementRequests()
      .then(async (restored) => {
        if (cancelled) return;
        for (const entry of restored) {
          const parsed = parseAgentManagementRequest(entry.payload);
          if (!parsed || parsed.requestId !== entry.requestId) continue;
          if (entry.disposition === "failed" && entry.result) {
            const result = entry.result as Record<string, unknown>;
            await publishAgentManagementAuditReceipt({
              requestId: entry.requestId,
              sourcePubkey: entry.sourcePubkey,
              requesterName: requesterNameFor(
                managedAgentsRef.current,
                entry.sourcePubkey,
              ),
              capability: entry.capability,
              target: `agent ${parsed.action}: ${String(result.error ?? "failed")}`,
              succeeded: false,
            }).catch(() => undefined);
            await publishAgentManagementResult({
              requestId: entry.requestId,
              sourcePubkey: entry.sourcePubkey,
              result: entry.result,
            });
            continue;
          }
          if (
            enqueueAgentManagementRequest(
              acceptedRequestsRef.current,
              seenRequestIds.current,
              { agentPubkey: entry.sourcePubkey, request: parsed },
            )
          ) {
            presentNextRequest();
          }
        }
      })
      .catch((cause) => {
        if (!cancelled) {
          setError(
            cause instanceof Error
              ? cause.message
              : "Could not restore pending agent drafts.",
          );
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  React.useEffect(() => {
    managedAgentsRef.current = managedAgentsQuery.data;
    channelsRef.current = channelsQuery.data;
    if (managedAgentsQuery.data && channelsQuery.data) {
      const buffered = bufferedRequestsRef.current.splice(0);
      for (const candidate of buffered) {
        acceptOwnedRequest(candidate.agentPubkey, candidate.request);
      }
    }
  }, [channelsQuery.data, managedAgentsQuery.data]);

  React.useEffect(
    () =>
      subscribeAgentManagementRequests((agentPubkey, next) => {
        // Observer frames are owner-scoped and authenticated. Any managed agent
        // this Desktop owns may draft a change; defer the ownership decision
        // until the managed-agent query has initialized so ephemeral requests
        // cannot disappear during startup.
        if (
          classifyAgentManagementOrigin(
            managedAgentsRef.current,
            channelsRef.current,
            agentPubkey,
            next.request.channelId,
          ) === "buffer"
        ) {
          bufferedRequestsRef.current.push({ agentPubkey, request: next });
          if (bufferedRequestsRef.current.length > 100) {
            bufferedRequestsRef.current.shift();
          }
          return;
        }
        acceptOwnedRequest(agentPubkey, next);
      }),
    [],
  );

  const matchingPersonas = React.useMemo(() => {
    if (request?.action !== "update") return [];
    const target = request.request.agentName.trim().toLocaleLowerCase();
    return (personasQuery.data ?? []).filter(
      (persona) =>
        persona.displayName.trim().toLocaleLowerCase() === target &&
        requestTargetsEditablePersona(persona),
    );
  }, [personasQuery.data, request]);
  const currentPersona =
    matchingPersonas.length === 1 ? matchingPersonas[0] : undefined;

  const isPending =
    createPersonaMutation.isPending ||
    updatePersonaMutation.isPending ||
    createAgentMutation.isPending ||
    globalConfigPending;

  function assertAgentCanActFromOrigin(channelId: string) {
    const targetChannel = (channelsQuery.data ?? []).find(
      (channel) => channel.id === channelId,
    );
    const requestingPubkey = sourceAgentPubkey.current?.toLowerCase();
    if (
      !targetChannel?.isMember ||
      !requestingPubkey ||
      !targetChannel.memberPubkeys.some(
        (pubkey) => pubkey.toLowerCase() === requestingPubkey,
      )
    ) {
      throw new Error(
        "An agent can only manage agents from a channel you both belong to.",
      );
    }
  }

  async function submitCreate(
    input: CreatePersonaInput | UpdatePersonaInput,
    intent: AgentCreateIntent,
    backendIntent: BackendIntent | null,
    _revealSecret = true,
    onCreated?: (
      created: import("@/shared/api/types").CreateManagedAgentResponse,
    ) => Promise<void>,
    onFailed?: (cause: unknown) => Promise<void>,
  ): Promise<boolean> {
    if (request?.action !== "create" || "id" in input) {
      return false;
    }
    setError(null);
    try {
      assertAgentCanActFromOrigin(request.request.channelId);
      const runtimes = await availableRuntimesForStart(runtimesQuery);
      const runtime = input.runtime
        ? runtimes.find((candidate) => candidate.id === input.runtime)
        : getDefaultPersonaRuntime(runtimes, globalConfig.preferred_runtime);
      if (!runtime) {
        throw new Error("Choose an available runtime for this agent.");
      }

      const avatarUrl = await resolveManagedAgentAvatarUrl(
        input.avatarUrl,
        undefined,
        runtime.avatarUrl,
      );
      const persona = await createPersonaMutation.mutateAsync({
        ...input,
        avatarUrl,
      });

      if (intent === "definition_start") {
        const created = await createAgentMutation.mutateAsync(
          await buildInstanceInputForDefinition(
            persona,
            runtime,
            undefined,
            backendIntent ?? undefined,
          ),
        );
        if (created.spawnError) throw new Error(created.spawnError);
        await onCreated?.(created);
        const targetChannel = (channelsQuery.data ?? []).find(
          (channel) => channel.id === request.request.channelId,
        );
        await createdAgentAttachment.presentCreatedAgent(created, {
          id: request.request.channelId,
          name: targetChannel?.name ?? "this channel",
        });
      }

      await Promise.all([
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
      ]);
      dismiss();
      return true;
    } catch (cause) {
      await onFailed?.(cause).catch(() => undefined);
      setError(
        cause instanceof Error ? cause.message : "Could not save this agent.",
      );
      return false;
    }
  }

  async function performUpdate(
    input: CreatePersonaInput | UpdatePersonaInput,
    onUpdated?: () => Promise<void>,
    onFailed?: (cause: unknown) => Promise<void>,
  ) {
    if (request?.action !== "update" || !("id" in input)) {
      return false;
    }
    setError(null);
    try {
      assertAgentCanActFromOrigin(request.request.channelId);
      await updatePersonaMutation.mutateAsync(input);
      await onUpdated?.();
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
      ]);
      dismiss();
      return true;
    } catch (cause) {
      await onFailed?.(cause).catch(() => undefined);
      setError(
        cause instanceof Error ? cause.message : "Could not save this agent.",
      );
      return false;
    }
  }

  async function submitUpdate(input: CreatePersonaInput | UpdatePersonaInput) {
    return performUpdate(input);
  }

  function dismiss() {
    const activeRequestId = pendingRequestId.current;
    const wasDelegatedApply = activeRequestId
      ? delegatedApplyIdsRef.current.delete(activeRequestId)
      : false;
    if (activeRequestId && !wasDelegatedApply) {
      void dismissAgentManagementRequest(activeRequestId).catch(() => {
        // A concurrent apply completion may already have resolved it.
      });
    }
    pendingRequestId.current = null;
    sourceAgentPubkey.current = null;
    setRequest(null);
    presentNextRequest();
  }

  const createInitialValues = React.useMemo(
    () =>
      request?.action === "create" ? createInputFromRequest(request) : null,
    [request],
  );

  const editInitialValues = React.useMemo(() => {
    if (request?.action !== "update" || !currentPersona) return null;
    const configuredRuntime = request.request.runtime ?? currentPersona.runtime;
    if (request.request.effort && !configuredRuntime && globalConfigLoading) {
      return null;
    }
    const requestedRuntime =
      configuredRuntime ??
      getDefaultPersonaRuntime(
        runtimesQuery.data ?? [],
        globalConfig.preferred_runtime,
      )?.id;
    const effortEnvVar = runtimesQuery.data?.find(
      (runtime) => runtime.id === requestedRuntime,
    )?.thinkingEnvVar;
    if (request.request.effort && !effortEnvVar) return null;
    return updateInputFromRequest(
      request,
      editPersonaDialogState(currentPersona)
        .initialValues as UpdatePersonaInput,
      effortEnvVar,
    );
  }, [
    currentPersona,
    globalConfig.preferred_runtime,
    globalConfigLoading,
    request,
    runtimesQuery.data,
  ]);

  const editEffortError = React.useMemo(() => {
    if (request?.action !== "update" || !request.request.effort) return null;
    if (runtimesQuery.isLoading) return null;
    const configuredRuntime =
      request.request.runtime ?? currentPersona?.runtime;
    if (!configuredRuntime && globalConfigLoading) return null;
    if (runtimesQuery.isError) {
      return "Could not load runtime metadata required to apply effort.";
    }
    const requestedRuntime =
      configuredRuntime ??
      getDefaultPersonaRuntime(
        runtimesQuery.data ?? [],
        globalConfig.preferred_runtime,
      )?.id;
    const runtime = runtimesQuery.data?.find(
      (candidate) => candidate.id === requestedRuntime,
    );
    if (!runtime) return "The selected runtime is not in the runtime catalog.";
    if (!runtime.thinkingEnvVar) {
      return "The selected runtime does not support effort configuration.";
    }
    return null;
  }, [
    currentPersona?.runtime,
    globalConfig.preferred_runtime,
    globalConfigLoading,
    request,
    runtimesQuery.data,
    runtimesQuery.isError,
    runtimesQuery.isLoading,
  ]);

  const editError = React.useMemo(() => {
    if (request?.action !== "update") return error;
    if (error) return error;
    if (editEffortError) return editEffortError;
    if (matchingPersonas.length > 1) {
      return "More than one personal agent has that name. Rename it in Agents, then ask the agent again.";
    }
    if (!currentPersona) {
      return "Agents can only update a personal agent profile by its current name.";
    }
    return null;
  }, [
    currentPersona,
    editEffortError,
    error,
    matchingPersonas.length,
    request,
  ]);

  async function completeCreateSuccess(
    requestId: string,
    sourcePubkey: string,
    created: import("@/shared/api/types").CreateManagedAgentResponse,
  ) {
    await completeAgentManagementRequest(requestId, true, {
      pubkey: created.agent.pubkey,
    });
    await publishAgentManagementAuditReceipt({
      requestId,
      sourcePubkey,
      requesterName: requesterNameFor(managedAgentsRef.current, sourcePubkey),
      capability: "agent_creation",
      target: `created agent ${created.agent.name} (${created.agent.pubkey})`,
      succeeded: true,
    });
    await publishAgentCreateSecretResponse({
      requestId,
      sourcePubkey,
      agentPubkey: created.agent.pubkey,
      privateKeyNsec: created.privateKeyNsec,
    });
  }

  async function completeCreateFailure(
    requestId: string,
    sourcePubkey: string,
    cause: unknown,
  ) {
    const message =
      cause instanceof Error ? cause.message : "Agent creation failed.";
    const result = { error: message };
    await completeAgentManagementRequest(requestId, false, result);
    await publishAgentManagementAuditReceipt({
      requestId,
      sourcePubkey,
      requesterName: requesterNameFor(managedAgentsRef.current, sourcePubkey),
      capability: "agent_creation",
      target: `agent create: ${message}`,
      succeeded: false,
    });
    await publishAgentManagementResult({
      requestId,
      sourcePubkey,
      result,
    });
  }

  async function submitReviewedCreate(
    input: CreatePersonaInput | UpdatePersonaInput,
    intent: AgentCreateIntent,
    backendIntent: BackendIntent | null,
  ): Promise<boolean> {
    const requestId = pendingRequestId.current;
    const sourcePubkey = sourceAgentPubkey.current;
    if (
      request?.action !== "create" ||
      !requestId ||
      !sourcePubkey ||
      intent !== "definition_start" ||
      delegatedApplyIdsRef.current.has(requestId)
    ) {
      return submitCreate(input, intent, backendIntent);
    }
    try {
      await beginOwnerReviewedAgentManagementRequest(requestId);
      // Reuse the apply-finalization path while the ledger itself retains the
      // owner-reviewed provenance for the audit wording.
      delegatedApplyIdsRef.current.add(requestId);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not approve this agent request.",
      );
      return false;
    }
    return submitCreate(
      input,
      intent,
      backendIntent,
      true,
      (created) => completeCreateSuccess(requestId, sourcePubkey, created),
      (cause) => completeCreateFailure(requestId, sourcePubkey, cause),
    );
  }

  const autoApproveCreate = React.useEffectEvent(
    async (requestId: string, input: CreatePersonaInput): Promise<void> => {
      if (request?.requestId !== requestId) return;
      const delegated = delegatedApplyIdsRef.current.has(requestId);
      const sourcePubkey = sourceAgentPubkey.current;
      const submitted = await submitCreate(
        input,
        "definition_start",
        null,
        false,
        delegated && sourcePubkey
          ? (created) => completeCreateSuccess(requestId, sourcePubkey, created)
          : undefined,
        delegated && sourcePubkey
          ? (cause) => completeCreateFailure(requestId, sourcePubkey, cause)
          : undefined,
      );
      if (!submitted && pendingRequestId.current === requestId) {
        setFailedAutoApprovalRequestId(requestId);
      }
    },
  );

  const autoApproveUpdate = React.useEffectEvent(
    async (requestId: string, input: UpdatePersonaInput): Promise<void> => {
      if (request?.requestId !== requestId || request.action !== "update")
        return;
      const sourcePubkey = sourceAgentPubkey.current;
      if (!sourcePubkey) return;
      if (
        capabilityForAgentManagementRequest(request) === "env_vars" &&
        request.request.envVars
      ) {
        setError(null);
        try {
          assertAgentCanActFromOrigin(request.request.channelId);
          const activation = await applyDelegatedPersonaEnvVarsRequest(
            request as Extract<AgentManagementRequest, { action: "update" }> & {
              request: { envVars: Record<string, string> };
            },
            input.id,
          );
          await completeDelegatedAgentUpdateSuccess(
            updateCompletionContext(requestId, sourcePubkey, request),
            activation,
          );
          await Promise.all([
            queryClient.invalidateQueries({ queryKey: personasQueryKey }),
            queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
          ]);
          dismiss();
        } catch (cause) {
          await completeDelegatedAgentUpdateFailure(
            updateCompletionContext(requestId, sourcePubkey, request),
            cause,
          ).catch(() => undefined);
          setError(
            cause instanceof Error
              ? cause.message
              : "Could not save this agent.",
          );
          if (pendingRequestId.current === requestId) {
            setFailedAutoApprovalRequestId(requestId);
          }
        }
        return;
      }
      const submitted = await performUpdate(
        input,
        () =>
          completeDelegatedAgentUpdateSuccess(
            updateCompletionContext(requestId, sourcePubkey, request),
          ),
        (cause) =>
          completeDelegatedAgentUpdateFailure(
            updateCompletionContext(requestId, sourcePubkey, request),
            cause,
          ),
      );
      if (!submitted && pendingRequestId.current === requestId) {
        setFailedAutoApprovalRequestId(requestId);
      }
    },
  );

  const failDelegatedUpdate = React.useEffectEvent(
    async (
      requestId: string,
      sourcePubkey: string,
      updateRequest: Extract<AgentManagementRequest, { action: "update" }>,
      message: string,
    ) => {
      await completeDelegatedAgentUpdateFailure(
        updateCompletionContext(requestId, sourcePubkey, updateRequest),
        new Error(message),
      );
    },
  );

  const applyGlobalConfigRequest = React.useEffectEvent(
    async (
      requestId: string,
      globalRequest: Extract<
        AgentManagementRequest,
        { action: "global_config" }
      >,
      ownerReviewed: boolean,
    ): Promise<void> => {
      if (
        request?.requestId !== requestId ||
        request.action !== "global_config"
      ) {
        return;
      }
      const sourcePubkey = sourceAgentPubkey.current;
      if (!sourcePubkey) return;
      setGlobalConfigPending(true);
      setError(null);
      try {
        assertAgentCanActFromOrigin(globalRequest.request.channelId);
        if (ownerReviewed) {
          await beginOwnerReviewedAgentManagementRequest(requestId);
          delegatedApplyIdsRef.current.add(requestId);
        }
        const result = await applyDelegatedGlobalConfig(globalRequest);
        queryClient.setQueryData(globalAgentConfigQueryKey, result.config);
        await completeDelegatedGlobalConfigSuccess({
          requestId,
          sourcePubkey,
          requesterName: requesterNameFor(
            managedAgentsRef.current,
            sourcePubkey,
          ),
          request: globalRequest,
          saveResult: result,
        });
        dismiss();
      } catch (cause) {
        await completeDelegatedGlobalConfigFailure({
          requestId,
          sourcePubkey,
          requesterName: requesterNameFor(
            managedAgentsRef.current,
            sourcePubkey,
          ),
          cause,
        }).catch(() => undefined);
        setError(
          cause instanceof Error
            ? cause.message
            : "Could not save global agent defaults.",
        );
        setFailedAutoApprovalRequestId(requestId);
      } finally {
        setGlobalConfigPending(false);
      }
    },
  );

  React.useEffect(() => {
    if (
      request?.action !== "create" ||
      !createInitialValues ||
      failedAutoApprovalRequestId === request.requestId ||
      autoApprovingRequestId.current === request.requestId
    ) {
      return;
    }
    const delegated = delegatedApplyIdsRef.current.has(request.requestId);
    const legacyAutoApprove =
      request.intent === "draft" && globalConfig.auto_approve_agent_drafts;
    if (!delegated && !legacyAutoApprove) return;
    const requestId = request.requestId;
    autoApprovingRequestId.current = requestId;
    void autoApproveCreate(requestId, createInitialValues).finally(() => {
      if (autoApprovingRequestId.current === requestId) {
        autoApprovingRequestId.current = null;
      }
    });
  }, [
    createInitialValues,
    failedAutoApprovalRequestId,
    globalConfig.auto_approve_agent_drafts,
    request,
  ]);

  React.useEffect(() => {
    if (
      request?.action !== "update" ||
      !editInitialValues ||
      !delegatedApplyIdsRef.current.has(request.requestId) ||
      failedAutoApprovalRequestId === request.requestId ||
      autoApprovingRequestId.current === request.requestId
    ) {
      return;
    }
    const requestId = request.requestId;
    autoApprovingRequestId.current = requestId;
    void autoApproveUpdate(requestId, editInitialValues).finally(() => {
      if (autoApprovingRequestId.current === requestId) {
        autoApprovingRequestId.current = null;
      }
    });
  }, [editInitialValues, failedAutoApprovalRequestId, request]);

  React.useEffect(() => {
    if (
      request?.action !== "update" ||
      !editEffortError ||
      !delegatedApplyIdsRef.current.has(request.requestId) ||
      failedAutoApprovalRequestId === request.requestId ||
      autoApprovingRequestId.current === request.requestId
    ) {
      return;
    }
    const requestId = request.requestId;
    const sourcePubkey = sourceAgentPubkey.current;
    if (!sourcePubkey) return;
    autoApprovingRequestId.current = requestId;
    void failDelegatedUpdate(
      requestId,
      sourcePubkey,
      request,
      editEffortError,
    ).finally(() => {
      setFailedAutoApprovalRequestId(requestId);
      if (autoApprovingRequestId.current === requestId) {
        autoApprovingRequestId.current = null;
      }
    });
  }, [editEffortError, failedAutoApprovalRequestId, request]);

  React.useEffect(() => {
    if (
      request?.action !== "global_config" ||
      !delegatedApplyIdsRef.current.has(request.requestId) ||
      failedAutoApprovalRequestId === request.requestId ||
      autoApprovingRequestId.current === request.requestId
    ) {
      return;
    }
    const requestId = request.requestId;
    autoApprovingRequestId.current = requestId;
    void applyGlobalConfigRequest(requestId, request, false).finally(() => {
      if (autoApprovingRequestId.current === requestId) {
        autoApprovingRequestId.current = null;
      }
    });
  }, [failedAutoApprovalRequestId, request]);

  const hideAutoApprovedCreate =
    request?.action === "create" &&
    (delegatedApplyIdsRef.current.has(request.requestId) ||
      (request.intent === "draft" && globalConfig.auto_approve_agent_drafts)) &&
    failedAutoApprovalRequestId !== request.requestId;
  const hideAutoApprovedUpdate =
    request?.action === "update" &&
    delegatedApplyIdsRef.current.has(request.requestId) &&
    !editEffortError &&
    failedAutoApprovalRequestId !== request.requestId;
  const hideAutoApprovedGlobalConfig =
    request?.action === "global_config" &&
    delegatedApplyIdsRef.current.has(request.requestId) &&
    failedAutoApprovalRequestId !== request.requestId;

  return {
    request:
      hideAutoApprovedCreate ||
      hideAutoApprovedUpdate ||
      hideAutoApprovedGlobalConfig
        ? null
        : request,
    createInitialValues,
    editInitialValues,
    editError,
    error,
    ...createdAgentAttachment,
    isPending,
    runtimes: runtimesQuery.data ?? [],
    runtimeCatalogStatus: runtimesQuery.isLoading
      ? ("loading" as const)
      : runtimesQuery.isError
        ? ("error" as const)
        : ("ready" as const),
    submitCreate: submitReviewedCreate,
    submitUpdate,
    approveGlobalConfig: () => {
      if (request?.action !== "global_config") return Promise.resolve();
      return applyGlobalConfigRequest(
        request.requestId,
        request,
        !delegatedApplyIdsRef.current.has(request.requestId),
      );
    },
    globalConfigPending,
    dismiss,
  };
}
