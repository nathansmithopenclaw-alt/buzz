import { useState } from "react";
import { ChevronDown, SlidersHorizontal } from "lucide-react";
import {
  useProviderDefaultAccount,
  useClaudeAccountChange,
} from "../accountChanges";
import {
  useSubscriptionAccountsQuery,
  useSubscriptionAccountsEnabledQuery,
  useSubscriptionAccountClock,
} from "../subscriptionAccounts";
import { subscriptionAccountExpiry } from "../subscriptionAccountExpiry";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from "@/shared/ui/dropdown-menu";
import { ClaudeAccountChangeDialog } from "./ClaudeAccountChangeDialog";

export function ClaudeDefaultAccountControl({
  harness = "claude",
  label = "Claude",
  available = true,
}: {
  harness?: string;
  label?: string;
  available?: boolean;
}) {
  const selected = useProviderDefaultAccount(harness);
  const accounts = useSubscriptionAccountsQuery();
  const enabled = useSubscriptionAccountsEnabledQuery();
  const change = useClaudeAccountChange();
  const now = useSubscriptionAccountClock();
  const [destination, setDestination] = useState<string>();
  const eligible =
    accounts.data?.filter(
      (account) =>
        account.harness === harness &&
        (harness !== "claude" || account.kind === "token") &&
        account.state === "ready" &&
        account.nickname &&
        !subscriptionAccountExpiry(account, now)?.expired,
    ) ?? [];
  return (
    <section
      className="space-y-3 rounded-xl border border-primary/20 bg-primary/5 p-4"
      aria-label={`Default ${label} account`}
    >
      <div className="flex items-center gap-2 text-sm font-medium">
        <SlidersHorizontal className="size-4 text-primary" aria-hidden="true" />
        <p>Default {label} account</p>
      </div>
      {available ? (
        <p className="text-xs text-muted-foreground">
          New {label === "OpenAI" ? "Codex" : label} agents use this account.
          Existing agents keep their choices until you select Use default.
          Changing the default updates only agents that follow it.
        </p>
      ) : null}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="outline"
            className="w-full justify-between"
            disabled={
              !available ||
              eligible.length === 0 ||
              !enabled.data ||
              selected.isPending ||
              selected.isError ||
              accounts.isPending ||
              accounts.isError ||
              Boolean(change.data && change.data.phase !== "complete")
            }
          >
            {!available
              ? "Not available yet"
              : (accounts.data?.find((account) => account.id === selected.data)
                  ?.nickname ??
                (selected.data
                  ? "Default account unavailable"
                  : "Choose default account"))}
            <ChevronDown className="size-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuRadioGroup
            value={selected.data ?? ""}
            onValueChange={(value) => {
              if (value !== selected.data) setDestination(value);
            }}
          >
            {eligible.map((account) => (
              <DropdownMenuRadioItem key={account.id} value={account.id}>
                {account.nickname}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      {available &&
      !accounts.isPending &&
      !accounts.isError &&
      eligible.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          Add and name a ready account to choose a default.
        </p>
      ) : null}
      {!available ? (
        <p className="text-xs text-muted-foreground">
          Grok account defaults will be available when account support is ready.
        </p>
      ) : null}
      {selected.isError ? (
        <p role="alert" className="text-sm text-destructive">
          Could not load the default account.{" "}
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void selected.refetch()}
          >
            Retry
          </Button>
        </p>
      ) : null}
      {destination ? (
        <ClaudeAccountChangeDialog
          request={{ kind: "default", destination }}
          onClose={() => setDestination(undefined)}
        />
      ) : null}
    </section>
  );
}
