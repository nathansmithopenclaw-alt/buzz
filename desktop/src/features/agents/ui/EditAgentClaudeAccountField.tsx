import type { ManagedAgent } from "@/shared/api/types";
import { ClaudeAccountPill } from "./ClaudeAccountPill";

export function EditAgentClaudeAccountField({
  agent,
}: {
  agent: ManagedAgent;
}) {
  if (!agent.supportsClaudeAccounts) return null;
  return (
    <div className="space-y-1.5">
      <p className="text-sm font-medium text-foreground">Claude account</p>
      <ClaudeAccountPill agent={agent} />
    </div>
  );
}
