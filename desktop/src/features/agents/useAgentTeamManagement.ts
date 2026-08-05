import * as React from "react";

import { createChannelManagedAgents } from "./channelAgents";
import { classifyAgentManagementOrigin } from "./agentManagementBuffer";
import { subscribeAgentTeamManagementRequests } from "./observerRelayStore";
import {
  useAvailableAcpRuntimes,
  useManagedAgentsQuery,
  usePersonasQuery,
} from "./hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import { getDefaultPersonaRuntime } from "./lib/resolvePersonaRuntime";
import { createTeam, listTeams, updateTeam } from "@/shared/api/tauriTeams";
import {
  beginOwnerReviewedAgentManagementRequest,
  completeAgentManagementRequest,
  dismissAgentManagementRequest,
  publishAgentManagementApplyingResult,
  publishAgentManagementAuditReceipt,
  publishAgentManagementDraftResult,
  publishAgentManagementResult,
  registerAgentManagementRequest,
  restoreAgentManagementRequests,
} from "@/shared/api/tauriManagedAgents";
import type { AgentPersona } from "@/shared/api/types";
import {
  parseAgentTeamManagementRequest,
  type AgentTeamManagementRequest,
} from "./agentTeamManagement";

type PendingTeamReview = {
  sourcePubkey: string;
  request: AgentTeamManagementRequest;
};

function resolveOne<T extends { id: string }>(
  values: readonly T[],
  reference: string,
  label: (value: T) => string,
  kind: string,
): T {
  const normalized = reference.trim().toLocaleLowerCase();
  const matches = values.filter(
    (value) =>
      value.id.toLocaleLowerCase() === normalized ||
      label(value).trim().toLocaleLowerCase() === normalized,
  );
  if (matches.length !== 1) {
    throw new Error(
      matches.length === 0
        ? `${kind} not found: ${reference}`
        : `${kind} reference is ambiguous: ${reference}`,
    );
  }
  return matches[0];
}

function resolvePersonaIds(
  personas: readonly AgentPersona[],
  members: readonly string[],
) {
  return members.map(
    (member) =>
      resolveOne(personas, member, (persona) => persona.displayName, "Agent")
        .id,
  );
}

/** Applies grantee-signed team operations only after Rust durably authorizes them. */
export function useAgentTeamManagement() {
  const personasQuery = usePersonasQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const channelsQuery = useChannelsQuery();
  const runtimesQuery = useAvailableAcpRuntimes();
  const queueRef = React.useRef(Promise.resolve());
  const pendingReviewsRef = React.useRef<PendingTeamReview[]>([]);
  const seenPendingIdsRef = React.useRef(new Set<string>());
  const [pendingReview, setPendingReview] =
    React.useState<PendingTeamReview | null>(null);
  const [reviewError, setReviewError] = React.useState<string | null>(null);
  const [isReviewPending, setIsReviewPending] = React.useState(false);
  const managedAgentsRef = React.useRef(managedAgentsQuery.data);
  const channelsRef = React.useRef(channelsQuery.data);
  const personasRef = React.useRef(personasQuery.data);
  const runtimesRef = React.useRef(runtimesQuery.data);
  const bufferedRequestsRef = React.useRef<PendingTeamReview[]>([]);

  React.useEffect(() => {
    managedAgentsRef.current = managedAgentsQuery.data;
    channelsRef.current = channelsQuery.data;
    personasRef.current = personasQuery.data;
    runtimesRef.current = runtimesQuery.data;
  }, [
    channelsQuery.data,
    managedAgentsQuery.data,
    personasQuery.data,
    runtimesQuery.data,
  ]);

  const presentNextReview = React.useEffectEvent(() => {
    setPendingReview(
      (current) => current ?? pendingReviewsRef.current.shift() ?? null,
    );
  });

  const enqueueReview = React.useEffectEvent((review: PendingTeamReview) => {
    if (seenPendingIdsRef.current.has(review.request.requestId)) return;
    seenPendingIdsRef.current.add(review.request.requestId);
    pendingReviewsRef.current.push(review);
    presentNextReview();
  });

  const apply = React.useEffectEvent(
    async (
      sourcePubkey: string,
      request: AgentTeamManagementRequest,
      ownerReviewed = false,
    ) => {
      if (
        classifyAgentManagementOrigin(
          managedAgentsRef.current,
          channelsRef.current,
          sourcePubkey,
          request.request.channelId,
        ) !== "accept"
      ) {
        return;
      }
      const registered = ownerReviewed
        ? ({ disposition: "apply" } as const)
        : await registerAgentManagementRequest({
            sourcePubkey,
            requestId: request.requestId,
            capability: "teams",
            intent: "apply",
            payload: request,
          });
      if (registered.disposition === "duplicate_applying") {
        await publishAgentManagementApplyingResult({
          requestId: request.requestId,
          sourcePubkey,
        });
        return;
      }
      if (
        (registered.disposition === "duplicate_applied" ||
          registered.disposition === "duplicate_failed") &&
        registered.result
      ) {
        const succeeded = registered.disposition === "duplicate_applied";
        const requesterName =
          managedAgentsRef.current?.find(
            (agent) =>
              agent.pubkey.toLowerCase() === sourcePubkey.toLowerCase(),
          )?.name ?? "delegated agent";
        const result = registered.result as Record<string, unknown>;
        await publishAgentManagementAuditReceipt({
          requestId: request.requestId,
          sourcePubkey,
          requesterName,
          capability: "teams",
          target: `team ${String(result.action ?? request.action)} ${String(result.teamId ?? result.error ?? "operation")}`,
          succeeded,
        });
        await publishAgentManagementResult({
          requestId: request.requestId,
          sourcePubkey,
          result: registered.result,
        });
        return;
      }
      if (
        registered.disposition === "draft" ||
        registered.disposition === "duplicate_draft"
      ) {
        await publishAgentManagementDraftResult({
          requestId: request.requestId,
          sourcePubkey,
        });
        enqueueReview({ sourcePubkey, request });
        return;
      }
      if (registered.disposition !== "apply") return;

      const personas = personasRef.current ?? [];
      const teams = await listTeams();
      let target = "team operation";
      let result: Record<string, unknown>;
      let completedSuccessfully = false;
      try {
        if (request.action === "create") {
          const team = await createTeam({
            name: request.request.name,
            description: request.request.description,
            instructions: request.request.instructions,
            personaIds: resolvePersonaIds(personas, request.request.members),
          });
          target = `created team ${team.name} (${team.id}), members 0→${team.personaIds.length}`;
          result = { action: "create", teamId: team.id, name: team.name };
        } else if (request.action === "update") {
          const current = resolveOne(
            teams,
            request.request.team,
            (team) => team.name,
            "Team",
          );
          const changes = [
            request.request.name !== undefined &&
            request.request.name !== current.name
              ? `name ${current.name}→${request.request.name}`
              : null,
            request.request.description !== undefined
              ? "description changed"
              : null,
            request.request.instructions !== undefined
              ? "instructions changed"
              : null,
            request.request.members !== undefined
              ? `members ${current.personaIds.length}→${request.request.members.length}`
              : null,
          ].filter((change): change is string => change !== null);
          const team = await updateTeam({
            id: current.id,
            name: request.request.name ?? current.name,
            description:
              request.request.description === undefined
                ? (current.description ?? undefined)
                : request.request.description,
            instructions:
              request.request.instructions === undefined
                ? (current.instructions ?? undefined)
                : request.request.instructions,
            personaIds:
              request.request.members === undefined
                ? current.personaIds
                : resolvePersonaIds(personas, request.request.members),
          });
          target = `updated team ${team.name} (${team.id}): ${changes.join(", ") || "no field changes"}`;
          result = { action: "update", teamId: team.id, name: team.name };
        } else {
          const team = resolveOne(
            teams,
            request.request.team,
            (candidate) => candidate.name,
            "Team",
          );
          const runtimes = runtimesRef.current ?? [];
          const fallback = getDefaultPersonaRuntime(runtimes);
          if (!fallback) throw new Error("No agent runtime is available.");
          const members = team.personaIds.map((personaId) =>
            resolveOne(
              personas,
              personaId,
              (persona) => persona.displayName,
              "Agent",
            ),
          );
          const deployed = await createChannelManagedAgents(
            request.request.channelId,
            members.map((persona) => {
              const runtime =
                runtimes.find(
                  (candidate) => candidate.id === persona.runtime,
                ) ?? fallback;
              return {
                runtime: {
                  id: runtime.id,
                  label: runtime.label,
                  command: runtime.command,
                  defaultArgs: runtime.defaultArgs,
                  mcpCommand: runtime.mcpCommand,
                },
                name: persona.displayName,
                systemPrompt: persona.systemPrompt,
                avatarUrl: persona.avatarUrl ?? undefined,
                model: persona.model ?? undefined,
                personaId: persona.id,
                teamId: team.id,
                forceNewInstance: true,
                role: "bot" as const,
              };
            }),
          );
          if (deployed.failures.length > 0) {
            throw new Error(
              `Team deployment had ${deployed.failures.length} failure(s): ${deployed.failures.map((failure) => failure.error).join("; ")}`,
            );
          }
          target = `deployed team ${team.name} (${team.id}) to channel ${request.request.channelId}: agents 0→${deployed.successes.length}`;
          result = {
            action: "add_to_channel",
            teamId: team.id,
            channelId: request.request.channelId,
            deployedCount: deployed.successes.length,
          };
        }
        await completeAgentManagementRequest(request.requestId, true, result);
        completedSuccessfully = true;
        const requesterName =
          managedAgentsRef.current?.find(
            (agent) =>
              agent.pubkey.toLowerCase() === sourcePubkey.toLowerCase(),
          )?.name ?? "delegated agent";
        await publishAgentManagementAuditReceipt({
          requestId: request.requestId,
          sourcePubkey,
          requesterName,
          capability: "teams",
          target,
          succeeded: true,
        });
        await publishAgentManagementResult({
          requestId: request.requestId,
          sourcePubkey,
          result,
        });
      } catch (cause) {
        if (completedSuccessfully) {
          // The mutation is durably applied. Leave audit/result delivery
          // retryable under the same request UUID instead of relabeling it as
          // a failed operation after a relay error.
          throw cause;
        }
        const message =
          cause instanceof Error ? cause.message : "Team apply failed.";
        const result = { error: message };
        await completeAgentManagementRequest(
          request.requestId,
          false,
          result,
        ).catch(() => undefined);
        const requesterName =
          managedAgentsRef.current?.find(
            (agent) =>
              agent.pubkey.toLowerCase() === sourcePubkey.toLowerCase(),
          )?.name ?? "delegated agent";
        await publishAgentManagementAuditReceipt({
          requestId: request.requestId,
          sourcePubkey,
          requesterName,
          capability: "teams",
          target: `${target}: ${message}`,
          succeeded: false,
        });
        await publishAgentManagementResult({
          requestId: request.requestId,
          sourcePubkey,
          result,
        }).catch(() => undefined);
      }
    },
  );

  React.useEffect(() => {
    if (!managedAgentsQuery.data || !channelsQuery.data) return;
    const buffered = bufferedRequestsRef.current.splice(0);
    for (const { sourcePubkey, request } of buffered) {
      queueRef.current = queueRef.current
        .then(() => apply(sourcePubkey, request))
        .catch((error) => {
          console.error("Delegated team operation failed:", error);
        });
    }
  }, [channelsQuery.data, managedAgentsQuery.data]);

  React.useEffect(() => {
    let cancelled = false;
    void restoreAgentManagementRequests()
      .then(async (restored) => {
        if (cancelled) return;
        for (const entry of restored) {
          const request = parseAgentTeamManagementRequest(entry.payload);
          if (!request || request.requestId !== entry.requestId) continue;
          if (entry.disposition === "failed" && entry.result) {
            const result = entry.result as Record<string, unknown>;
            const requesterName =
              managedAgentsRef.current?.find(
                (agent) =>
                  agent.pubkey.toLowerCase() ===
                  entry.sourcePubkey.toLowerCase(),
              )?.name ?? "delegated agent";
            await publishAgentManagementAuditReceipt({
              requestId: entry.requestId,
              sourcePubkey: entry.sourcePubkey,
              requesterName,
              capability: "teams",
              target: `team operation: ${String(result.error ?? "failed")}`,
              succeeded: false,
            }).catch(() => undefined);
            await publishAgentManagementResult({
              requestId: entry.requestId,
              sourcePubkey: entry.sourcePubkey,
              result: entry.result,
            });
            continue;
          }
          enqueueReview({ sourcePubkey: entry.sourcePubkey, request });
        }
      })
      .catch((cause) => {
        if (!cancelled) {
          setReviewError(
            cause instanceof Error
              ? cause.message
              : "Could not restore pending team requests.",
          );
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  React.useEffect(
    () =>
      subscribeAgentTeamManagementRequests((sourcePubkey, request) => {
        const origin = classifyAgentManagementOrigin(
          managedAgentsRef.current,
          channelsRef.current,
          sourcePubkey,
          request.request.channelId,
        );
        if (origin === "buffer") {
          bufferedRequestsRef.current.push({ sourcePubkey, request });
          if (bufferedRequestsRef.current.length > 100) {
            bufferedRequestsRef.current.shift();
          }
          return;
        }
        if (origin !== "accept") return;
        queueRef.current = queueRef.current
          .then(() => apply(sourcePubkey, request))
          .catch((error) => {
            console.error("Delegated team operation failed:", error);
          });
      }),
    [],
  );

  async function approvePendingReview() {
    if (!pendingReview || isReviewPending) return;
    setIsReviewPending(true);
    setReviewError(null);
    try {
      await beginOwnerReviewedAgentManagementRequest(
        pendingReview.request.requestId,
      );
      await apply(pendingReview.sourcePubkey, pendingReview.request, true);
      setPendingReview(null);
      presentNextReview();
    } catch (cause) {
      setReviewError(
        cause instanceof Error
          ? cause.message
          : "Could not apply team request.",
      );
    } finally {
      setIsReviewPending(false);
    }
  }

  async function dismissPendingReview() {
    if (!pendingReview || isReviewPending) return;
    setIsReviewPending(true);
    setReviewError(null);
    try {
      await dismissAgentManagementRequest(pendingReview.request.requestId);
      setPendingReview(null);
      presentNextReview();
    } catch (cause) {
      setReviewError(
        cause instanceof Error
          ? cause.message
          : "Could not dismiss team request.",
      );
    } finally {
      setIsReviewPending(false);
    }
  }

  return {
    pendingReview,
    reviewError,
    isReviewPending,
    approvePendingReview,
    dismissPendingReview,
  };
}
