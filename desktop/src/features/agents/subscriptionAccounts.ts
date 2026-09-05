import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  getSubscriptionAccountsEnabled,
  listSubscriptionAccounts,
  subscriptionAccountCapabilities,
} from "@/shared/api/tauriSubscriptionAccounts";

export const subscriptionAccountsQueryKey = ["subscription-accounts"] as const;
export const subscriptionAccountCapabilitiesQueryKey = [
  "subscription-account-capabilities",
] as const;

/** Both selection surfaces and Settings observe this same lifecycle store. */
export function useSubscriptionAccountsQuery() {
  return useQuery({
    queryKey: subscriptionAccountsQueryKey,
    queryFn: listSubscriptionAccounts,
    staleTime: 0,
    // Provider login happens in another app. Keep observing its completion
    // even when WebKit still reports Buzz as unfocused on return.
    refetchIntervalInBackground: true,
    refetchOnWindowFocus: true,
    refetchInterval: (query) =>
      query.state.data?.some((account) => account.state === "connecting")
        ? 1000
        : 30_000,
  });
}

export function useSubscriptionAccountCapabilitiesQuery() {
  return useQuery({
    queryKey: subscriptionAccountCapabilitiesQueryKey,
    queryFn: subscriptionAccountCapabilities,
    staleTime: 60_000,
  });
}

export const subscriptionAccountsEnabledQueryKey = [
  "subscription-accounts-enabled",
] as const;
export function useSubscriptionAccountsEnabledQuery() {
  return useQuery({
    queryKey: subscriptionAccountsEnabledQueryKey,
    queryFn: getSubscriptionAccountsEnabled,
    staleTime: 0,
  });
}

/** Keep visible expiry trackers current while a settings/profile panel stays open. */
export function useSubscriptionAccountClock() {
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const update = () => setNow(Date.now() / 1000);
    const interval = setInterval(update, 1000);
    window.addEventListener("focus", update);
    return () => {
      clearInterval(interval);
      window.removeEventListener("focus", update);
    };
  }, []);
  return now;
}
