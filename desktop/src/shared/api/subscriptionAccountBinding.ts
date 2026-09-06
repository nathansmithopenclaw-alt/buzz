/** A saved selection and a ready runtime are separate facts. */
export type SubscriptionAccountBinding = {
  subscriptionAccountId?: string | null;
  subscriptionAccountInheritDefault?: boolean;
  subscriptionAccountGeneration?: number;
  subscriptionAccountHarness?: string | null;
  runningSubscriptionAccountId?: string | null;
  runningSubscriptionAccountGeneration?: number | null;
  subscriptionAccountError?: string | null;
};
export type RawSubscriptionAccountBinding = {
  subscription_account_id?: string | null;
  subscription_account_inherit_default?: boolean;
  subscription_account_generation?: number;
  subscription_account_harness?: string | null;
  running_subscription_account_id?: string | null;
  running_subscription_account_generation?: number | null;
  subscription_account_error?: string | null;
};
export function subscriptionAccountBindingFromRaw(
  value: RawSubscriptionAccountBinding,
): SubscriptionAccountBinding {
  return {
    subscriptionAccountId: value.subscription_account_id ?? null,
    subscriptionAccountInheritDefault:
      value.subscription_account_inherit_default ?? false,
    subscriptionAccountGeneration: value.subscription_account_generation ?? 0,
    subscriptionAccountHarness: value.subscription_account_harness ?? null,
    runningSubscriptionAccountId: value.running_subscription_account_id ?? null,
    runningSubscriptionAccountGeneration:
      value.running_subscription_account_generation ?? null,
    subscriptionAccountError: value.subscription_account_error ?? null,
  };
}
