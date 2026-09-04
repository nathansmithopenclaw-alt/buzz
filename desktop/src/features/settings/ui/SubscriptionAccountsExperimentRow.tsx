import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  subscriptionAccountsEnabledQueryKey,
  useSubscriptionAccountsEnabledQuery,
} from "@/features/agents/subscriptionAccounts";
import { setSubscriptionAccountsEnabled } from "@/shared/api/tauriSubscriptionAccounts";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionRow } from "./SettingsOptionGroup";

export function SubscriptionAccountsExperimentRow() {
  const enabled = useSubscriptionAccountsEnabledQuery();
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: setSubscriptionAccountsEnabled,
    onSuccess: (_, value) =>
      client.setQueryData(subscriptionAccountsEnabledQueryKey, value),
  });
  return (
    <SettingsOptionRow>
      <div className="min-w-0 flex-1">
        <p
          className="text-sm font-medium"
          id="subscription-accounts-experiment-label"
        >
          Subscription Accounts
        </p>
        <p className="text-xs text-muted-foreground/70">
          Choose the subscription account each local agent uses. Turning this
          off preserves existing selections and recovery controls.
        </p>
        {enabled.isError || mutation.isError ? (
          <p role="alert" className="text-xs text-destructive">
            Could not update Subscription Accounts.{" "}
            {String(mutation.error ?? enabled.error)}
          </p>
        ) : null}
      </div>
      <Switch
        aria-labelledby="subscription-accounts-experiment-label"
        checked={enabled.data ?? false}
        disabled={enabled.isPending || enabled.isError || mutation.isPending}
        onCheckedChange={(value) => mutation.mutate(value)}
      />
    </SettingsOptionRow>
  );
}
