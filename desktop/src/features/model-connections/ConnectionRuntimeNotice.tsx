import type { ModelConnection } from "./api";

export function ConnectionRuntimeNotice({
  runtimeId,
  connection,
}: {
  runtimeId: string;
  connection: ModelConnection | undefined;
}) {
  if (!connection || !["codex", "buzz-agent", "goose"].includes(runtimeId))
    return null;
  const context = connection.qualification?.context_window;
  if (runtimeId === "buzz-agent" || runtimeId === "goose") {
    return (
      <p
        className="text-xs text-muted-foreground"
        data-testid="connection-runtime-policy"
      >
        {context
          ? `Context: ${context.toLocaleString()} tokens; output limit: ${Math.min(8192, Math.max(256, Math.floor(context / 8)), Math.floor(context / 2)).toLocaleString()}. `
          : "Test this connection to establish its context window. "}
        {runtimeId === "goose"
          ? "Uses native Goose tools and Buzz session tools with a keyless connection. "
          : "Uses Buzz session tools only. "}{" "}
        Skips user and project MCP servers, account apps, and computer-use for
        this text-only connection.
      </p>
    );
  }
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
