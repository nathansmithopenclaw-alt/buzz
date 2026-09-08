import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  applyClaudeAccountChange,
  previewClaudeAccountChange,
  type ClaudeAccountChangeRequest,
} from "@/shared/api/tauriSubscriptionAccounts";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { claudeAccountChangeQueryKey } from "../accountChanges";
import { ClaudeAccountChangeProgress } from "./ClaudeAccountChangeProgress";
import {
  useSubscriptionAccountsQuery,
  useSubscriptionAccountClock,
} from "../subscriptionAccounts";
import { subscriptionAccountExpiry } from "../subscriptionAccountExpiry";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from "@/shared/ui/dropdown-menu";
import { ChevronDown } from "lucide-react";

export function ClaudeAccountChangeDialog({
  request: initial,
  allowAll = false,
  onClose,
}: {
  request: ClaudeAccountChangeRequest;
  allowAll?: boolean;
  onClose: () => void;
}) {
  const [request, setRequest] = useState(initial);
  const [submitted, setSubmitted] = useState(false);
  const accounts = useSubscriptionAccountsQuery();
  const now = useSubscriptionAccountClock();
  const client = useQueryClient();
  const preview = useQuery({
    queryKey: ["claude-account-preview", request],
    queryFn: () => previewClaudeAccountChange(request),
    enabled: !submitted,
    retry: false,
  });
  const apply = useMutation({
    mutationKey: ["apply-claude-account-change"],
    mutationFn: () => {
      if (!preview.data)
        throw new Error("Review the affected agents before continuing");
      setSubmitted(true);
      return applyClaudeAccountChange(request, preview.data.fingerprint);
    },
    onSuccess: (result) => {
      if (result.phase === "complete") onClose();
    },
    onError: () => {
      setSubmitted(false);
      void preview.refetch();
    },
    onSettled: () =>
      client.invalidateQueries({ queryKey: claudeAccountChangeQueryKey }),
  });
  const destination =
    request.kind === "remove" ? request.replacement : request.destination;
  const destinationName =
    accounts.data?.find((account) => account.id === destination)?.nickname ??
    "the selected account";
  const title =
    request.kind === "remove"
      ? "Remove this account?"
      : request.kind === "default"
        ? `Change the default ${accounts.data?.find((account) => account.id === destination)?.harness === "codex" ? "OpenAI" : "Claude"} account?`
        : "Change subscription account?";
  const operation = preview.data?.operation;
  const running =
    operation?.members.reduce((sum, member) => sum + member.running_count, 0) ??
    0;
  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <AlertDialogContent>
        <AlertDialogTitle>{title}</AlertDialogTitle>
        <AlertDialogDescription>
          {request.kind === "remove" && !destination
            ? "Stop and detach this account’s agents. Unfinished work stays held until you choose another named account. The token is removed only after every agent is detached."
            : `Use ${destinationName}. Buzz will pause affected running agents, switch accounts, and continue their saved work. Stopped agents stay stopped.`}
        </AlertDialogDescription>
        {request.kind === "remove" && !submitted ? (
          <div className="space-y-2">
            <p className="text-sm font-medium">
              What should happen to this account’s agents?
            </p>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" className="w-full justify-between">
                  {destination
                    ? `Move to ${destinationName}`
                    : "Stop agents and detach account"}
                  <ChevronDown className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent>
                <DropdownMenuRadioGroup
                  value={request.replacement ?? "__stop__"}
                  onValueChange={(value) =>
                    setRequest({
                      ...request,
                      replacement: value === "__stop__" ? null : value,
                    })
                  }
                >
                  {accounts.data
                    ?.filter(
                      (account) =>
                        account.id !== request.account_id &&
                        account.harness === "claude" &&
                        account.kind === "token" &&
                        account.state === "ready" &&
                        account.nickname &&
                        !subscriptionAccountExpiry(account, now)?.expired,
                    )
                    .map((account) => (
                      <DropdownMenuRadioItem
                        key={account.id}
                        value={account.id}
                      >
                        Move to {account.nickname}
                      </DropdownMenuRadioItem>
                    ))}
                  <DropdownMenuRadioItem value="__stop__">
                    Stop agents and detach account
                  </DropdownMenuRadioItem>
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        ) : null}
        {request.kind === "agent" && allowAll && !submitted ? (
          <fieldset
            aria-label="Account change scope"
            className="flex flex-wrap gap-2"
          >
            <Button
              variant={!request.all_on_current ? "default" : "outline"}
              aria-pressed={!request.all_on_current}
              onClick={() => setRequest({ ...request, all_on_current: false })}
            >
              This agent
            </Button>
            <Button
              variant={request.all_on_current ? "default" : "outline"}
              aria-pressed={request.all_on_current}
              onClick={() => setRequest({ ...request, all_on_current: true })}
            >
              All agents on the current account
            </Button>
          </fieldset>
        ) : null}
        {!submitted && operation ? (
          <div className="space-y-2 text-sm">
            <p>
              {operation.members.length} agents on this Mac · {running} running
              instances across communities.
            </p>
            {operation.change_default ? (
              <p>
                {destination
                  ? "The default account changes too."
                  : "The default account will be cleared."}
              </p>
            ) : null}
            <ul className="max-h-48 space-y-1 overflow-auto">
              {operation.members.map((member) => (
                <li key={member.pubkey} className="break-words">
                  {member.name} ·{" "}
                  {member.running_count
                    ? `${member.running_count} running`
                    : "No restart needed"}
                </li>
              ))}
            </ul>
          </div>
        ) : null}
        {preview.isPending && !submitted ? (
          <p role="status">Checking affected agents…</p>
        ) : null}
        {preview.isError ? <p role="alert">{String(preview.error)}</p> : null}
        {apply.isError ? <p role="alert">{String(apply.error)}</p> : null}
        {submitted ? <ClaudeAccountChangeProgress /> : null}
        <AlertDialogFooter>
          <Button variant="outline" onClick={onClose}>
            {submitted ? "Close" : "Cancel"}
          </Button>
          {!submitted ? (
            <Button
              disabled={!preview.data || preview.isFetching || apply.isPending}
              onClick={() => apply.mutate()}
            >
              {request.kind === "remove"
                ? destination
                  ? "Move agents and remove"
                  : "Stop agents and remove"
                : "Apply account change"}
            </Button>
          ) : null}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
