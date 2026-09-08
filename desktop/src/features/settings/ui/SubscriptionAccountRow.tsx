import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useClaudeAccountChange } from "@/features/agents/accountChanges";
import {
  subscriptionAccountExpiry,
  subscriptionAccountNeedsRenewal,
} from "@/features/agents/subscriptionAccountExpiry";
import {
  useSubscriptionAccountsQuery,
  subscriptionAccountsQueryKey,
} from "@/features/agents/subscriptionAccounts";
import { ClaudeAccountChangeDialog } from "@/features/agents/ui/ClaudeAccountChangeDialog";
import {
  beginSubscriptionAccountToken,
  beginSubscriptionAccountLogin,
  cancelSubscriptionAccountLogin,
  renameSubscriptionAccount,
  removeSubscriptionAccount,
  type SubscriptionAccount,
} from "@/shared/api/tauriSubscriptionAccounts";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
const stateLabels: Record<SubscriptionAccount["state"], string> = {
  connecting: "Waiting for browser login and verification…",
  ready: "Subscription login checked",
  needs_login: "Needs login",
  removal_pending: "Removal incomplete — retry cleanup",
  unavailable: "Unavailable",
};

export function SubscriptionAccountRow({
  account,
  now,
  loginAvailable,
}: {
  loginAvailable: boolean | undefined;
  account: SubscriptionAccount;
  now: number;
}) {
  const token = account.kind === "token";
  const accounts = useSubscriptionAccountsQuery();
  const group = useClaudeAccountChange();
  const groupPending = Boolean(group.data && group.data.phase !== "complete");
  const replacement = accounts.data?.find(
    (other) =>
      other.id !== account.id &&
      other.harness === "claude" &&
      other.kind === "token" &&
      other.state === "ready" &&
      other.nickname &&
      !subscriptionAccountExpiry(other, now)?.expired,
  );
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
      className="space-y-3 rounded-xl border border-border/60 bg-card p-4"
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
        ) : account.state !== "removal_pending" &&
          (token || loginAvailable !== false) ? (
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
            disabled={mutation.isPending || groupPending}
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
      {!token && loginAvailable === false ? (
        <p className="text-sm text-muted-foreground">
          This sign-in method has been retired. Move its agents to a long-lived
          token, then remove this account.
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
      {confirmRemove && account.harness === "claude" ? (
        <ClaudeAccountChangeDialog
          request={{
            kind: "remove",
            account_id: account.id,
            replacement: replacement?.id ?? null,
          }}
          onClose={() => setConfirmRemove(false)}
        />
      ) : confirmRemove ? (
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
