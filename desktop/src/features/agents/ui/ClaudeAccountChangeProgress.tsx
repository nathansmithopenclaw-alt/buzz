import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ChevronDown } from "lucide-react";
import {
  useClaudeAccountChange,
  claudeAccountChangeQueryKey,
} from "../accountChanges";
import {
  useSubscriptionAccountsQuery,
  useSubscriptionAccountClock,
} from "../subscriptionAccounts";
import { subscriptionAccountExpiry } from "../subscriptionAccountExpiry";
import { retryClaudeAccountChange } from "@/shared/api/tauriSubscriptionAccounts";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from "@/shared/ui/dropdown-menu";

/** Persistent recovery is available after dismissing any initiating dialog. */
export function ClaudeAccountChangeProgress({ pubkey }: { pubkey?: string }) {
  const operation = useClaudeAccountChange();
  const accounts = useSubscriptionAccountsQuery();
  const now = useSubscriptionAccountClock();
  const client = useQueryClient();
  const retry = useMutation({
    mutationKey: ["retry-claude-account-change"],
    mutationFn: (destination: string | null) => {
      if (!operation.data) throw new Error("Account change unavailable");
      return retryClaudeAccountChange(operation.data.id, destination);
    },
    onSettled: () =>
      client.invalidateQueries({ queryKey: claudeAccountChangeQueryKey }),
  });
  if (operation.isError)
    return (
      <p role="alert" className="text-sm text-destructive">
        Could not load account change progress. {String(operation.error)}{" "}
        <Button
          variant="ghost"
          size="sm"
          onClick={() => void operation.refetch()}
        >
          Retry status
        </Button>
      </p>
    );
  const data = operation.data;
  if (
    !data ||
    data.phase === "complete" ||
    (pubkey && !data.members.some((member) => member.pubkey === pubkey))
  )
    return null;
  const paused = data.phase === "paused";
  return (
    <section
      className="space-y-2 rounded-md border p-3 text-sm"
      aria-label="Account change progress"
    >
      <p role="status">
        {paused
          ? "Account change needs attention"
          : data.phase === "removing"
            ? "Removing the detached account…"
            : "Changing accounts and continuing saved work…"}
      </p>
      <p>
        {data.members.filter((member) => member.done).length} of{" "}
        {data.members.length} agents updated.
      </p>
      <ul className="max-h-48 space-y-1 overflow-auto">
        {data.members.map((member) => (
          <li key={member.pubkey} className="break-words">
            {member.name}:{" "}
            {member.error ?? (member.done ? "Updated" : "Pending")}
          </li>
        ))}
      </ul>
      {data.error ? <p role="alert">{data.error}</p> : null}
      {paused ? (
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            disabled={retry.isPending}
            onClick={() => retry.mutate(null)}
          >
            Retry account change
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button size="sm" variant="outline" disabled={retry.isPending}>
                Choose another account <ChevronDown className="size-3" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent>
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
                  <DropdownMenuItem
                    key={account.id}
                    onSelect={() => retry.mutate(account.id)}
                  >
                    {account.nickname}
                  </DropdownMenuItem>
                ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      ) : null}
      {retry.isError ? <p role="alert">{String(retry.error)}</p> : null}
    </section>
  );
}
