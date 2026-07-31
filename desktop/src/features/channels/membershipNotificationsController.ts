import { getChannelIdFromTags } from "@/features/messages/lib/threading";
import { refreshChannelsWhenIdle } from "@/features/channels/refreshChannelsWhenIdle";
import type {
  RelayLiveSubscriptionOptions,
  RelaySubscriptionFilter,
} from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_MEMBER_ADDED_NOTIFICATION,
  KIND_MEMBER_REMOVED_NOTIFICATION,
  KIND_NIP29_GROUP_MEMBERS,
} from "@/shared/constants/kinds";
import { createTrailingDebounce } from "@/shared/lib/trailingDebounce";

const CHANNELS_INVALIDATE_DEBOUNCE_MS = 250;
const RETRY_BASE_MS = 1_000;
const RETRY_MAX_MS = 30_000;

type Dispose = () => Promise<void>;

type TimerHost = {
  setTimeout: (handler: () => void, ms: number) => number;
  clearTimeout: (id: number) => void;
};

export type MembershipNotificationsRelay = {
  subscribeLive: (
    filter: RelaySubscriptionFilter,
    onEvent: (event: RelayEvent) => void,
    options?: RelayLiveSubscriptionOptions,
  ) => Promise<Dispose>;
  subscribeToReconnects: (listener: () => void) => () => void;
};

export type MembershipNotificationsController = {
  ready: Promise<void>;
  stop: () => void;
};

export function startMembershipNotifications({
  currentPubkey,
  invalidateChannelMembership,
  invalidateChannels,
  isChannelsFetching,
  relay,
  timerHost = window,
  nowSeconds = () => Math.floor(Date.now() / 1_000),
  logError = (error) =>
    console.error("Failed to subscribe to membership notifications", error),
}: {
  currentPubkey: string;
  invalidateChannelMembership: (channelId: string) => void;
  invalidateChannels: () => void;
  isChannelsFetching: () => number;
  relay: MembershipNotificationsRelay;
  timerHost?: TimerHost;
  nowSeconds?: () => number;
  logError?: (error: unknown) => void;
}): MembershipNotificationsController {
  let cancelled = false;
  let retryAttempt = 0;
  let retryTimeout: number | undefined;
  let running = false;
  let rerunRequested = false;
  let activeDisposes: Dispose[] = [];

  const channelsInvalidation = createTrailingDebounce(
    () =>
      refreshChannelsWhenIdle({
        isFetching: isChannelsFetching,
        invalidate: invalidateChannels,
        reArm: () => channelsInvalidation.trigger(),
      }),
    CHANNELS_INVALIDATE_DEBOUNCE_MS,
    timerHost,
  );

  const handleEvent = (event: RelayEvent) => {
    channelsInvalidation.trigger();
    if (
      event.kind !== KIND_MEMBER_ADDED_NOTIFICATION &&
      event.kind !== KIND_MEMBER_REMOVED_NOTIFICATION
    ) {
      return;
    }
    const channelId = getChannelIdFromTags(event.tags);
    if (channelId) {
      invalidateChannelMembership(channelId);
    }
  };

  const clearRetry = () => {
    if (retryTimeout === undefined) return;
    timerHost.clearTimeout(retryTimeout);
    retryTimeout = undefined;
  };

  const requestRearm = () => {
    if (cancelled) return;
    rerunRequested = true;
    if (!running) {
      void runLoop();
    }
  };

  const scheduleRetry = () => {
    if (cancelled || retryTimeout !== undefined) return;
    const delayMs = Math.min(RETRY_BASE_MS * 2 ** retryAttempt, RETRY_MAX_MS);
    retryAttempt += 1;
    retryTimeout = timerHost.setTimeout(() => {
      retryTimeout = undefined;
      requestRearm();
    }, delayMs);
  };

  const disposeActive = async () => {
    const disposes = activeDisposes;
    activeDisposes = [];
    await Promise.all(disposes.map((dispose) => dispose().catch(() => {})));
  };

  const subscribeAll = async () => {
    const since = nowSeconds();
    const filters: RelaySubscriptionFilter[] = [
      {
        kinds: [
          KIND_MEMBER_ADDED_NOTIFICATION,
          KIND_MEMBER_REMOVED_NOTIFICATION,
        ],
        "#p": [currentPubkey],
        limit: 50,
        since: since - 30,
      },
      {
        kinds: [KIND_NIP29_GROUP_MEMBERS],
        "#p": [currentPubkey],
        limit: 50,
        since,
      },
    ];
    const nextDisposes: Dispose[] = [];
    try {
      for (const filter of filters) {
        nextDisposes.push(
          await relay.subscribeLive(filter, handleEvent, {
            onTerminalClosed: scheduleRetry,
          }),
        );
      }
      if (cancelled) {
        await Promise.all(
          nextDisposes.map((dispose) => dispose().catch(() => {})),
        );
        return false;
      }
      activeDisposes = nextDisposes;
      retryAttempt = 0;
      return true;
    } catch (error) {
      await Promise.all(
        nextDisposes.map((dispose) => dispose().catch(() => {})),
      );
      logError(error);
      return false;
    }
  };

  async function runLoop() {
    if (running || cancelled) return;
    running = true;
    try {
      do {
        rerunRequested = false;
        await disposeActive();
        if (cancelled) return;
        const subscribed = await subscribeAll();
        if (!subscribed && !cancelled) {
          scheduleRetry();
        }
      } while (rerunRequested && !cancelled);
    } finally {
      running = false;
      if (rerunRequested && !cancelled) {
        void runLoop();
      }
    }
  }

  const unsubscribeReconnect = relay.subscribeToReconnects(() => {
    channelsInvalidation.trigger();
    clearRetry();
    retryAttempt = 0;
    requestRearm();
  });
  const ready = runLoop();

  return {
    ready,
    stop: () => {
      cancelled = true;
      clearRetry();
      channelsInvalidation.cancel();
      unsubscribeReconnect();
      void disposeActive();
    },
  };
}
