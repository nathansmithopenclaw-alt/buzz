export type ClaudeAccountSource =
  | "instance"
  | "definition"
  | "global"
  | "credentials_file";

export type ClaudeAccountBinding = {
  claudeAccountId: string | null;
  claudeAccountSource: ClaudeAccountSource;
  claudeAccountShadowed: boolean;
  supportsClaudeAccounts: boolean;
};

export type RawClaudeAccountBinding = {
  claude_account_id?: string | null;
  claude_account_source?: ClaudeAccountSource;
  claude_account_shadowed?: boolean;
  supports_claude_accounts?: boolean;
};

export function claudeAccountBindingFromRaw(
  value: RawClaudeAccountBinding,
): ClaudeAccountBinding {
  return {
    claudeAccountId: value.claude_account_id ?? null,
    claudeAccountSource: value.claude_account_source ?? "credentials_file",
    claudeAccountShadowed: value.claude_account_shadowed ?? false,
    supportsClaudeAccounts: value.supports_claude_accounts ?? false,
  };
}
