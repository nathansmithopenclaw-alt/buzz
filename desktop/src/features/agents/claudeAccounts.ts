import { useQuery } from "@tanstack/react-query";

import { listClaudeAccounts } from "@/shared/api/tauriClaudeAccounts";

export const claudeAccountsQueryKey = ["claude-accounts"] as const;

export function useClaudeAccountsQuery() {
  return useQuery({
    queryKey: claudeAccountsQueryKey,
    queryFn: listClaudeAccounts,
    staleTime: 60_000,
  });
}
