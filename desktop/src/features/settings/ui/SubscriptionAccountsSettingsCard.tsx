import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ClaudeDefaultAccountControl } from "@/features/agents/ui/ClaudeDefaultAccountControl";
import { ClaudeAccountChangeProgress } from "@/features/agents/ui/ClaudeAccountChangeProgress";
import {
  subscriptionAccountsQueryKey,
  useSubscriptionAccountsQuery,
  useSubscriptionAccountsEnabledQuery,
  useSubscriptionAccountCapabilitiesQuery,
  useSubscriptionAccountClock,
} from "@/features/agents/subscriptionAccounts";
import {
  beginSubscriptionAccountToken,
  beginSubscriptionAccountLogin,
} from "@/shared/api/tauriSubscriptionAccounts";
import { Button } from "@/shared/ui/button";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { SubscriptionAccountRow } from "./SubscriptionAccountRow";

const providers = [
  {
    provider: "anthropic",
    harness: "claude",
    title: "Anthropic",
    label: "Claude",
    description: "Claude subscriptions and long-lived tokens.",
  },
  {
    provider: "openai",
    harness: "codex",
    title: "OpenAI",
    label: "OpenAI",
    description: "ChatGPT subscriptions for Codex agents.",
  },
  {
    provider: "xai",
    harness: "grok",
    title: "Grok",
    label: "Grok",
    description: "xAI subscriptions for Grok agents.",
  },
] as const;

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
  const addingDisabled =
    !enabled.data ||
    connecting ||
    add.isPending ||
    accounts.isError ||
    accounts.isPending;
  return (
    <section
      className="min-w-0 space-y-6"
      data-testid="settings-subscription-accounts"
    >
      <SectionHeader
        title="Subscription Accounts"
        description="Manage your accounts by provider. Choose a default for new agents, or select a different account for an individual agent."
      />
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
      <ClaudeAccountChangeProgress />
      {accounts.isPending ? (
        <p role="status">Loading accounts…</p>
      ) : accounts.isError ? (
        <div role="alert">
          <p>Could not load subscription accounts. {String(accounts.error)}</p>
          <Button variant="outline" onClick={() => void accounts.refetch()}>
            Retry loading accounts
          </Button>
        </div>
      ) : null}
      {capabilities.isPending ? (
        <p role="status">Checking providers…</p>
      ) : capabilities.isError ? (
        <div role="alert">
          <p>Could not check provider support.</p>
          <Button variant="outline" onClick={() => void capabilities.refetch()}>
            Retry providers
          </Button>
        </div>
      ) : null}
      {providers.map(({ provider, harness, title, label, description }) => {
        const saved =
          accounts.data?.filter((account) => account.provider === provider) ??
          [];
        const capability = capabilities.data?.find(
          (item) => item.harness === harness,
        );
        return (
          <section
            key={provider}
            aria-labelledby={`subscription-provider-${provider}`}
            className="space-y-4 border-t border-border/60 pt-6"
          >
            <div className="flex items-start justify-between gap-3">
              <div className="space-y-1">
                <h3
                  id={`subscription-provider-${provider}`}
                  className="text-base font-semibold"
                >
                  {title} accounts
                </h3>
                <p className="text-sm text-muted-foreground">{description}</p>
              </div>
              {!accounts.isPending && !accounts.isError ? (
                <span className="shrink-0 rounded-full bg-muted px-2.5 py-1 text-xs text-muted-foreground">
                  {saved.length} saved
                </span>
              ) : null}
            </div>
            <ClaudeDefaultAccountControl
              harness={harness}
              label={label}
              available={Boolean(
                capability?.supported || capability?.token_supported,
              )}
            />
            <fieldset
              className="space-y-3"
              aria-label={`${title} saved accounts`}
            >
              {saved.map((account) => (
                <SubscriptionAccountRow
                  key={account.id}
                  account={account}
                  now={now}
                  loginAvailable={capability?.supported}
                />
              ))}
              {!accounts.isPending &&
              !accounts.isError &&
              saved.length === 0 ? (
                <p className="rounded-xl border border-dashed border-border/60 p-4 text-sm text-muted-foreground">
                  No {title} accounts saved.
                </p>
              ) : null}
            </fieldset>
            {capability ? (
              <div className="space-y-2">
                <div className="flex flex-wrap gap-2">
                  {capability.token_supported ? (
                    <Button
                      variant="outline"
                      disabled={addingDisabled}
                      onClick={() => add.mutate({ harness, token: true })}
                    >
                      Add Claude long-lived token
                    </Button>
                  ) : null}
                  {!capability.token_supported || capability.supported ? (
                    <Button
                      variant="outline"
                      disabled={addingDisabled || !capability.supported}
                      onClick={() => add.mutate({ harness })}
                    >
                      Sign in with {title}
                      {harness === "codex" ? " (Codex)" : ""}
                    </Button>
                  ) : null}
                </div>
                {harness === "claude" ? (
                  <p className="text-xs text-muted-foreground">
                    To add a different Claude account, sign into that account on
                    claude.ai first. Check the account shown before authorizing.
                  </p>
                ) : null}
                {!capability.supported && capability.reason ? (
                  <p className="text-xs text-muted-foreground">
                    {capability.reason}
                  </p>
                ) : null}
              </div>
            ) : null}
          </section>
        );
      })}
      {connecting ? (
        <p role="status" className="text-xs text-muted-foreground">
          Complete or cancel the current login before starting another.
        </p>
      ) : null}
      {add.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {String(add.error)}
        </p>
      ) : null}
      <p className="text-xs text-muted-foreground">
        Credentials stay on this machine. Login checks confirm a local
        subscription login, not available quota or billing. Long-lived tokens
        expire after 365 days.
      </p>
    </section>
  );
}
