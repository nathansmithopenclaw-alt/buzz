import type { ModelConnection } from "./api";

export function ConnectionRuntimeNotice({
  runtimeId,
  connection,
}: {
  runtimeId: string;
  connection: ModelConnection | undefined;
}) {
  if (runtimeId !== "codex" || !connection) return null;
  const context = connection.qualification?.context_window;
  return (
    <p
      className="text-xs text-muted-foreground"
      data-testid="connection-runtime-policy"
    >
      {context
        ? `Context: ${context.toLocaleString()} tokens; auto-compaction: ${Math.floor((context * 3) / 4).toLocaleString()}. `
        : "Test this connection to establish its context window. "}
      Uses a separate Codex home. Skips user and project MCP servers, account
      apps, global skills and plugins, and computer-use for this text-only
      connection.
    </p>
  );
}
