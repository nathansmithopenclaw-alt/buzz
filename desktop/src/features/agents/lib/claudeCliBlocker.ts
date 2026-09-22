export type ClaudeCliBlocker = {
  code: number;
  model: string;
  installed: string;
  required: string;
  message: string;
  binaryPath: string;
};

export const CLAUDE_CLI_TOO_OLD_ERROR_CODE = -32550;

export function shouldCheckClaudeCli(
  runtimeId: string,
  model: string,
): boolean {
  return runtimeId === "claude" && model.trim() === "claude-opus-5-5";
}
