import {
  subscriptionAccountExpiry,
  subscriptionAccountNeedsRenewal,
} from "@/features/agents/subscriptionAccountExpiry";
import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  subscriptionAccountsQueryKey,
  useSubscriptionAccountCapabilitiesQuery,
  useSubscriptionAccountsEnabledQuery,
  useSubscriptionAccountsQuery,
  useSubscriptionAccountClock,
} from "@/features/agents/subscriptionAccounts";
import {
  beginSubscriptionAccountLogin,
  beginSubscriptionAccountToken,
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

function AccountRow({
  account,
  now,
}: {
  account: SubscriptionAccount;
  now: number;
}) {
  const token = account.kind === "token";
  const expiry = subscriptionAccountExpiry(account, now);
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
      if (token)
        return beginSubscriptionAccountToken(account.id).then(() => {});
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
          {account.provider} · {token ? "Long-lived token" : account.harness}
        </p>
        <p role="status" className="text-sm">
          {token && account.state === "ready"
            ? expiry?.expired
              ? "Renewal needed"
              : "Token ready"
            : token && account.state === "connecting"
              ? "Waiting for browser approval…"
              : token && account.state === "needs_login"
                ? "Renewal needed"
                : stateLabels[account.state]}
        </p>
        {expiry ? (
          <p data-testid={`token-expiry-${account.id}`} className="text-sm">
            {expiry.label}
            {account.expires_at != null
              ? ` · ${new Date(account.expires_at * 1000).toLocaleString()}`
              : ""}
          </p>
        ) : null}
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
            {token ? "Cancel token setup" : "Cancel login"}
          </Button>
        ) : account.state !== "removal_pending" ? (
          <Button
            variant="outline"
            disabled={mutation.isPending}
            onClick={() => mutation.mutate("reconnect")}
          >
            {token ? "Renew" : "Reconnect"}
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
      {subscriptionAccountNeedsRenewal(account, now) ? (
        <p
          role="note"
          data-testid={`token-renewal-notice-${account.id}`}
          className="text-sm text-destructive"
        >
          {expiry?.expired
            ? "This token has expired."
            : "This token expires within 30 days."}{" "}
          {account.state === "connecting"
            ? "Complete renewal in your browser to keep its agents running."
            : "Choose Renew to keep its agents running."}
        </p>
      ) : null}
      {token ? (
        <p className="text-xs text-muted-foreground">
          Renew in your browser. After approval, only this account’s agents
          restart. Buzz can show local OS notifications at 30, 14, 7 and 1 days,
          and at expiry, while Desktop is running. Notification permission is
          requested when the first reminder is due.
        </p>
      ) : null}
      {confirmRemove ? (
        <div className="space-y-2">
          <p className="text-sm">
            {token ? "Remove" : "Sign out and remove"} {label} from this
            machine? Accounts still selected or running cannot be removed.
          </p>
          <Button
            variant="destructive"
            disabled={mutation.isPending}
            onClick={() => mutation.mutate("remove")}
          >
            {token ? "Remove token" : "Sign out and remove"}
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
  const now = useSubscriptionAccountClock();
  const capabilities = useSubscriptionAccountCapabilitiesQuery();
  const enabled = useSubscriptionAccountsEnabledQuery();
  const connecting =
    accounts.data?.some((account) => account.state === "connecting") ?? false;
  const add = useMutation({
    mutationFn: ({ harness, token }: { harness: string; token?: boolean }) =>
      token
        ? beginSubscriptionAccountToken()
        : beginSubscriptionAccountLogin(harness),
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
        description="Sign in through your provider or approve a Claude long-lived token in your browser, then choose a nickname. Credentials stay on this machine."
      />
      <p className="text-xs text-muted-foreground">
        Login checks confirm a local subscription login. Tokens are stored
        securely by Buzz and expire after 365 days. These checks do not verify a
        model turn, available quota, or billing. Browser login expires
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
          <AccountRow key={account.id} account={account} now={now} />
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
                onClick={() => add.mutate({ harness: capability.harness })}
              >
                Sign in with {capability.provider} ({capability.harness})
              </Button>
              {capability.token_supported ? (
                <Button
                  variant="outline"
                  disabled={
                    !enabled.data ||
                    connecting ||
                    add.isPending ||
                    accounts.isError ||
                    accounts.isPending
                  }
                  onClick={() =>
                    add.mutate({ harness: capability.harness, token: true })
                  }
                >
                  Add Claude long-lived token
                </Button>
              ) : null}
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
