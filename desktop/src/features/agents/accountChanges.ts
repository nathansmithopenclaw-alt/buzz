import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getClaudeAccountChangeStatus,
  getClaudeDefaultAccount,
} from "@/shared/api/tauriSubscriptionAccounts";
import { subscriptionAccountsQueryKey } from "./subscriptionAccounts";

export const claudeDefaultAccountQueryKey = ["claude-default-account"] as const;
export const claudeAccountChangeQueryKey = ["claude-account-change"] as const;
export function useClaudeDefaultAccount() {
  return useQuery({
    queryKey: claudeDefaultAccountQueryKey,
    queryFn: getClaudeDefaultAccount,
  });
}
export function useClaudeAccountChange() {
  const client = useQueryClient();
  const query = useQuery({
    queryKey: claudeAccountChangeQueryKey,
    queryFn: getClaudeAccountChangeStatus,
    refetchInterval: 750,
  });
  const revision = JSON.stringify(query.data);
  useEffect(() => {
    if (revision && revision !== "null") {
      void client.invalidateQueries({ queryKey: ["managed-agents"] });
      void client.invalidateQueries({ queryKey: claudeDefaultAccountQueryKey });
      void client.invalidateQueries({ queryKey: subscriptionAccountsQueryKey });
    }
  }, [client, revision]);
  return query;
}
