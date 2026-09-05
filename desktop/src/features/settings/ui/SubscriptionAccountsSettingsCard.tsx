import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  subscriptionAccountsQueryKey,
  useSubscriptionAccountCapabilitiesQuery,
  useSubscriptionAccountsEnabledQuery,
  useSubscriptionAccountsQuery,
} from "@/features/agents/subscriptionAccounts";
import {
  beginSubscriptionAccountLogin,
  cancelSubscriptionAccountLogin,
  removeSubscriptionAccount,
  renameSubscriptionAccount,
  type SubscriptionAccount,
} from "@/shared/api/tauriSubscriptionAccounts";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SectionHeader } from "@/shared/ui/PageHeader";

const stateLabels: Record<SubscriptionAccount["state"], string> = {
  connecting: "Waiting for browser login and verification…",
  ready: "Subscription login checked",
  needs_login: "Needs login",
  removal_pending: "Removal incomplete — retry cleanup",
  unavailable: "Unavailable",
};

function AccountRow({ account }: { account: SubscriptionAccount }) {
  const client = useQueryClient();
  const [nickname, setNickname] = React.useState(account.nickname ?? "");
  const [confirmRemove, setConfirmRemove] = React.useState(false);
  const mutation = useMutation({
    mutationFn: (action: "reconnect" | "cancel" | "rename" | "remove") => {
      if (action === "remove") return removeSubscriptionAccount(account.id);
      if (action === "rename")
        return renameSubscriptionAccount(account.id, nickname).then(() => {});
      if (action === "cancel")
        return cancelSubscriptionAccountLogin(account.id).then(() => {});
      return beginSubscriptionAccountLogin(account.harness, account.id).then(
        () => {},
      );
    },
    onSuccess: async () => {
      setConfirmRemove(false);
      await client.invalidateQueries({
        queryKey: subscriptionAccountsQueryKey,
      });
    },
  });
  const label = account.nickname ?? `${account.provider} account`;
  return (
    <article
      className="space-y-3 rounded-lg border border-border/60 p-3"
      aria-label={label}
    >
      <div>
        <p className="font-medium">{label}</p>
        <p className="text-xs text-muted-foreground">
          {account.provider} · {account.harness}
        </p>
        <p role="status" className="text-sm">
          {stateLabels[account.state]}
        </p>
        {account.last_checked_at ? (
          <p className="text-xs text-muted-foreground">
            Checked {new Date(account.last_checked_at).toLocaleString()}
          </p>
        ) : null}
        {account.last_error ? (
          <p className="text-sm text-destructive">{account.last_error}</p>
        ) : null}
      </div>
      {account.state === "ready" ? (
        <form
          className="flex items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            mutation.mutate("rename");
          }}
        >
          <Input
            aria-label={`Nickname for ${label}`}
            placeholder="Choose a nickname"
            value={nickname}
            onChange={(event) => setNickname(event.target.value)}
            maxLength={80}
            disabled={mutation.isPending}
          />
          <Button
            type="submit"
            variant="outline"
            disabled={
              mutation.isPending ||
              !nickname.trim() ||
              nickname.trim() === account.nickname
            }
          >
            Save nickname
          </Button>
        </form>
      ) : null}
      <div className="flex flex-wrap gap-2">
        {account.state === "connecting" ? (
          <Button
            variant="outline"
            disabled={mutation.isPending}
            onClick={() => mutation.mutate("cancel")}
          >
            Cancel login
          </Button>
        ) : account.state !== "removal_pending" ? (
          <Button
            variant="outline"
            disabled={mutation.isPending}
            onClick={() => mutation.mutate("reconnect")}
          >
            Reconnect
          </Button>
        ) : null}
        {account.state !== "connecting" ? (
          <Button
            variant="ghost"
            disabled={mutation.isPending}
            onClick={() => setConfirmRemove(true)}
          >
            {account.state === "removal_pending" ? "Retry removal" : "Remove"}
          </Button>
        ) : null}
      </div>
      {confirmRemove ? (
        <div className="space-y-2">
          <p className="text-sm">
            Sign out and remove {label} from this machine? Accounts still
            selected or running cannot be removed.
          </p>
          <Button
            variant="destructive"
            disabled={mutation.isPending}
            onClick={() => mutation.mutate("remove")}
          >
            Sign out and remove
          </Button>{" "}
          <Button
            variant="ghost"
            disabled={mutation.isPending}
            onClick={() => setConfirmRemove(false)}
          >
            Keep account
          </Button>
        </div>
      ) : null}
      {mutation.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {String(mutation.error)}
        </p>
      ) : null}
    </article>
  );
}

export function SubscriptionAccountsSettingsCard() {
  const client = useQueryClient();
  const accounts = useSubscriptionAccountsQuery();
  const capabilities = useSubscriptionAccountCapabilitiesQuery();
  const enabled = useSubscriptionAccountsEnabledQuery();
  const connecting =
    accounts.data?.some((account) => account.state === "connecting") ?? false;
  const add = useMutation({
    mutationFn: (harness: string) => beginSubscriptionAccountLogin(harness),
    onSuccess: async () =>
      client.invalidateQueries({ queryKey: subscriptionAccountsQueryKey }),
  });
  return (
    <section
      className="min-w-0 space-y-4"
      data-testid="settings-subscription-accounts"
    >
      <SectionHeader
        title="Subscription Accounts"
        description="Sign in through your provider, verify the subscription login, then choose a nickname. Credentials stay with the harness on this machine."
      />
      <p className="text-xs text-muted-foreground">
        Readiness checks confirm a local subscription login. They do not verify
        a model turn, available quota, or billing. Browser login expires
        automatically if it is not completed.
      </p>
      {enabled.isError ? (
        <p role="alert" className="text-sm text-destructive">
          Could not load the experiment setting. {String(enabled.error)}
        </p>
      ) : enabled.data === false ? (
        <p className="text-sm text-muted-foreground">
          Enable Subscription Accounts in Experiments to add or select accounts.
          Existing selections remain enforced; recovery stays available.
        </p>
      ) : null}
      {accounts.isPending ? (
        <p role="status">Loading accounts…</p>
      ) : accounts.isError ? (
        <div role="alert">
          <p>Could not load subscription accounts. {String(accounts.error)}</p>
          <Button variant="outline" onClick={() => void accounts.refetch()}>
            Retry loading accounts
          </Button>
        </div>
      ) : accounts.data.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No subscription accounts saved.
        </p>
      ) : (
        accounts.data.map((account) => (
          <AccountRow key={account.id} account={account} />
        ))
      )}
      <div className="space-y-2">
        <p className="text-sm font-medium">Add account</p>
        <p className="text-xs text-muted-foreground">
          Your browser may already be signed into a provider account. To add a
          different Claude account, sign into that account on claude.ai first,
          then start the login here. Check the account shown before authorizing.
        </p>
        {capabilities.isPending ? (
          <p role="status">Checking providers…</p>
        ) : capabilities.isError ? (
          <div role="alert">
            <p>Could not check provider support.</p>
            <Button
              variant="outline"
              onClick={() => void capabilities.refetch()}
            >
              Retry providers
            </Button>
          </div>
        ) : (
          capabilities.data.map((capability) => (
            <div key={capability.harness} className="space-y-1">
              <Button
                variant="outline"
                disabled={
                  !enabled.data ||
                  !capability.supported ||
                  connecting ||
                  add.isPending ||
                  accounts.isError ||
                  accounts.isPending
                }
                onClick={() => add.mutate(capability.harness)}
              >
                Sign in with {capability.provider} ({capability.harness})
              </Button>
              {!capability.supported ? (
                <p className="text-xs text-muted-foreground">
                  {capability.reason ?? "This provider is not available yet."}
                </p>
              ) : null}
            </div>
          ))
        )}
        {connecting ? (
          <p className="text-xs text-muted-foreground">
            Complete or cancel the current login before starting another.
          </p>
        ) : null}
        {add.isError ? (
          <p role="alert" className="text-sm text-destructive">
            {String(add.error)}
          </p>
        ) : null}
      </div>
    </section>
  );
}
