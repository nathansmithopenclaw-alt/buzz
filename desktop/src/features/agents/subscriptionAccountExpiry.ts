import type { SubscriptionAccount } from "@/shared/api/tauriSubscriptionAccounts";

/** Token timestamps are UNIX seconds; round up partial remaining days. */
export function subscriptionAccountExpiry(
  account: SubscriptionAccount | undefined,
  nowSeconds = Date.now() / 1000,
): { label: string; expired: boolean } | null {
  if (account?.kind !== "token") return null;
  if (account.expires_at == null || !Number.isFinite(account.expires_at))
    return { label: "Expiry unavailable", expired: false };
  const remaining = account.expires_at - nowSeconds;
  if (remaining <= 0) return { label: "Expired", expired: true };
  const days = Math.ceil(remaining / 86400);
  return {
    label: `Expires in ${days} ${days === 1 ? "day" : "days"}`,
    expired: false,
  };
}

/** Keep renewal actionable even when local OS notifications are unavailable. */
export function subscriptionAccountNeedsRenewal(
  account: SubscriptionAccount,
  nowSeconds = Date.now() / 1000,
): boolean {
  return (
    account.kind === "token" &&
    account.expires_at != null &&
    Number.isFinite(account.expires_at) &&
    account.expires_at - nowSeconds <= 30 * 86400
  );
}
