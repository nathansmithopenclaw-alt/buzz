import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { Button } from "@/shared/ui/button";
import {
  type ClaudeCliBlocker,
  shouldCheckClaudeCli,
} from "../lib/claudeCliBlocker";
import { useAgentRunLocation } from "./AgentRunLocationContext";

export function ClaudeCliVersionBlocker({
  disabled,
  model,
  runtimeId,
}: {
  disabled: boolean;
  model: string;
  runtimeId: string;
}) {
  const runLocation = useAgentRunLocation();
  const [blocker, setBlocker] = useState<ClaudeCliBlocker | null>(null);
  const [updating, setUpdating] = useState(false);
  const [resultText, setResultText] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    if (runLocation === "remote" || !shouldCheckClaudeCli(runtimeId, model)) {
      setBlocker(null);
      return () => {
        active = false;
      };
    }
    void invoke<ClaudeCliBlocker | null>("get_claude_cli_blocker", { model })
      .then((value) => {
        if (active) setBlocker(value);
      })
      .catch(() => {
        if (active) setBlocker(null);
      });
    return () => {
      active = false;
    };
  }, [model, runLocation, runtimeId]);

  const update = async () => {
    setUpdating(true);
    setResultText(null);
    try {
      const result = await invoke<{
        installed?: string | null;
        restartedAgents: number;
        restartFailures: string[];
      }>("update_claude_code");
      const next = await invoke<ClaudeCliBlocker | null>(
        "get_claude_cli_blocker",
        { model },
      );
      setBlocker(next);
      const restartSuffix = result.restartFailures.length
        ? ` Restart failed: ${result.restartFailures.join("; ")}`
        : result.restartedAgents
          ? ` Restarted ${result.restartedAgents} affected agent${result.restartedAgents === 1 ? "" : "s"}.`
          : "";
      setResultText(
        next
          ? next.message
          : `Claude Code updated${result.installed ? ` to ${result.installed}` : ""}.${restartSuffix}`,
      );
    } catch (error) {
      setResultText(String(error));
    } finally {
      setUpdating(false);
    }
  };

  return (
    <>
      {blocker ? (
        <div className="space-y-2 rounded-md border border-destructive/40 bg-destructive/5 p-3">
          <p className="text-sm text-destructive">{blocker.message}</p>
          <p className="text-xs text-muted-foreground">
            Updating changes the global Claude Code install used by other Buzz
            agents and may affect other apps using the same install.
          </p>
          <Button
            disabled={disabled || updating}
            onClick={() => void update()}
            size="sm"
            type="button"
            variant="outline"
          >
            {updating ? "Updating Claude Code…" : "Update Claude Code"}
          </Button>
        </div>
      ) : null}
      {resultText ? (
        <p className="text-xs text-muted-foreground">{resultText}</p>
      ) : null}
    </>
  );
}
