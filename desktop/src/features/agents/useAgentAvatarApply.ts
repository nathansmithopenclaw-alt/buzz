import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { managedAgentsQueryKey, personasQueryKey } from "./hooks";
import { subscribeAgentAvatarApplyRequests } from "./observerRelayStore";
import { evictUsersBatchEntries } from "@/features/profile/hooks";
import { applyManagedAgentAvatarRequest } from "@/shared/api/tauriManagedAgents";

/** Owner-authorized avatar applies bypass the draft queue by design. */
export function useAgentAvatarApply() {
  const queryClient = useQueryClient();
  const seenRequestIds = React.useRef(new Set<string>());

  React.useEffect(
    () =>
      subscribeAgentAvatarApplyRequests(async (sourcePubkey, request) => {
        if (seenRequestIds.current.has(request.requestId)) return;
        seenRequestIds.current.add(request.requestId);
        try {
          const result = await applyManagedAgentAvatarRequest(
            sourcePubkey,
            request.authorizationEvent,
          );
          if (!result.applied) return;
          evictUsersBatchEntries(queryClient, [result.targetPubkey]);
          await Promise.all([
            queryClient.invalidateQueries({ queryKey: personasQueryKey }),
            queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
            queryClient.invalidateQueries({
              queryKey: ["user-profile", result.targetPubkey.toLowerCase()],
            }),
            queryClient.invalidateQueries({ queryKey: ["users-batch"] }),
          ]);
        } catch (error) {
          seenRequestIds.current.delete(request.requestId);
          console.error("Owner-authorized agent avatar apply failed:", error);
        }
      }),
    [queryClient],
  );
}
