import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { channelsQueryKey } from "@/features/channels/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { startMembershipNotifications } from "@/features/channels/membershipNotificationsController";

export function useMembershipNotifications(currentPubkey?: string) {
  const queryClient = useQueryClient();
  const normalizedCurrentPubkey = currentPubkey?.trim().toLowerCase() ?? "";

  React.useEffect(() => {
    if (normalizedCurrentPubkey.length === 0) {
      return;
    }
    return startMembershipNotifications({
      currentPubkey: normalizedCurrentPubkey,
      invalidateChannels: () => {
        void queryClient.invalidateQueries({ queryKey: channelsQueryKey });
      },
      isChannelsFetching: () =>
        queryClient.isFetching({ queryKey: channelsQueryKey, exact: true }),
      invalidateChannelMembership: (channelId) => {
        void queryClient.invalidateQueries({
          queryKey: ["channels", channelId, "detail"],
        });
        void queryClient.invalidateQueries({
          queryKey: ["channels", channelId, "members"],
        });
      },
      relay: relayClient,
    }).stop;
  }, [normalizedCurrentPubkey, queryClient]);
}
