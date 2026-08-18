import * as React from "react";

import {
  resolveProfileActivityAgent,
  type ProfileActivityAgent,
} from "@/features/profile/lib/profileActivityAgent";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import type { ProfilePanelView } from "./UserProfilePanelUtils";

export function useProfilePanelActivity({
  effectivePubkey,
  isBot,
  managedAgent,
  profile,
  relayAgent,
  setView,
  targetKey,
  viewerIsOwner,
}: {
  effectivePubkey: string | null;
  isBot: boolean;
  managedAgent: ManagedAgent | undefined;
  profile: { avatarUrl?: string | null; displayName?: string | null } | null;
  relayAgent: RelayAgent | undefined;
  setView: (view: ProfilePanelView) => void;
  targetKey: string;
  viewerIsOwner: boolean;
}) {
  const [channelId, setChannelId] = React.useState<string | null>(null);
  const activityAgent = React.useMemo<ProfileActivityAgent | null>(
    () =>
      resolveProfileActivityAgent({
        effectivePubkey,
        isBot,
        managedAgent,
        profile,
        relayAgent,
        viewerIsOwner,
      }),
    [effectivePubkey, isBot, managedAgent, profile, relayAgent, viewerIsOwner],
  );

  React.useEffect(() => {
    void targetKey;
    setChannelId(null);
  }, [targetKey]);
  const openActivity = React.useCallback(
    (nextChannelId?: string | null) => {
      setChannelId(nextChannelId ?? null);
      setView("activity");
    },
    [setView],
  );

  return {
    activityAgent,
    canViewActivity: viewerIsOwner && activityAgent !== null,
    channelId,
    openActivity,
  };
}
