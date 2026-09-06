import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  claudeAccountsQueryKey,
  useClaudeAccountsQuery,
} from "@/features/agents/claudeAccounts";
import { deleteClaudeAccount } from "@/shared/api/tauriClaudeAccounts";
import { Button } from "@/shared/ui/button";

/** Keep legacy bindings recoverable until explicit token selection succeeds. */
export function ClaudeAccountsSettingsCard() {
  const client = useQueryClient();
  const accounts = useClaudeAccountsQuery();
  const remove = useMutation({
    mutationFn: deleteClaudeAccount,
    onSuccess: async () =>
      client.invalidateQueries({ queryKey: claudeAccountsQueryKey }),
  });
  if (accounts.isPending || accounts.data?.length === 0) return null;
  return (
    <section className="space-y-3" data-testid="settings-claude-accounts">
      <p className="text-sm font-medium">Legacy Claude accounts</p>
      <p className="text-sm text-muted-foreground">
        Existing selections remain in place. Add and name a long-lived token
        above, then select it for each agent before removing the old entry.
      </p>
      {accounts.isError ? (
        <p role="alert">
          Could not load legacy accounts. {String(accounts.error)}
        </p>
      ) : (
        accounts.data?.map((account) => (
          <div
            key={account.id}
            className="flex items-center justify-between gap-2 rounded-lg border p-3"
          >
            <span>{account.label}</span>
            <Button
              variant="outline"
              disabled={remove.isPending}
              onClick={() => remove.mutate(account.id)}
            >
              Remove {account.label}
            </Button>
          </div>
        ))
      )}
      {remove.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {String(remove.error)}
        </p>
      ) : null}
    </section>
  );
}
