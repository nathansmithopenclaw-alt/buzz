import { useState } from "react";
import { ChevronDown } from "lucide-react";
import {
  useClaudeDefaultAccount,
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

export function ClaudeDefaultAccountControl() {
  const selected = useClaudeDefaultAccount();
  const accounts = useSubscriptionAccountsQuery();
  const enabled = useSubscriptionAccountsEnabledQuery();
  const change = useClaudeAccountChange();
  const now = useSubscriptionAccountClock();
  const [destination, setDestination] = useState<string>();
  return (
    <section
      className="space-y-2 rounded-lg border p-3"
      aria-label="Default Claude account"
    >
      <p className="text-sm font-medium">Default Claude account</p>
      <p className="text-xs text-muted-foreground">
        New Claude agents use this account. Existing agents keep their choices
        until you select Use default. Changing the default updates only agents
        that follow it.
      </p>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="outline"
            className="w-full justify-between"
            disabled={
              !enabled.data ||
              selected.isPending ||
              selected.isError ||
              accounts.isPending ||
              accounts.isError ||
              Boolean(change.data && change.data.phase !== "complete")
            }
          >
            {accounts.data?.find((account) => account.id === selected.data)
              ?.nickname ??
              (selected.data
                ? "Default account unavailable"
                : "Choose default account")}
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
            {accounts.data
              ?.filter(
                (account) =>
                  account.harness === "claude" &&
                  account.kind === "token" &&
                  account.state === "ready" &&
                  account.nickname &&
                  !subscriptionAccountExpiry(account, now)?.expired,
              )
              .map((account) => (
                <DropdownMenuRadioItem key={account.id} value={account.id}>
                  {account.nickname}
                </DropdownMenuRadioItem>
              ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
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
