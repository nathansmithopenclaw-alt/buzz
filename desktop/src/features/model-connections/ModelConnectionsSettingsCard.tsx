import { ConnectionHealth } from "./ConnectionHealth";
import { useEffect, useState } from "react";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";
import { ConnectModelDialog } from "./ConnectModelDialog";
import {
  listConnections,
  removeConnection,
  testConnection,
  type ModelConnection,
} from "./api";
export function ModelConnectionsSettingsCard() {
  const [rows, setRows] = useState<ModelConnection[]>([]);
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<ModelConnection>();
  const [removing, setRemoving] = useState("");
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const upsert = (row: ModelConnection) =>
    setRows((old) => [...old.filter((r) => r.id !== row.id), row]);
  useEffect(() => {
    let live = true;
    listConnections()
      .then((rows) => {
        if (live) setRows(rows);
      })
      .catch((error) => {
        if (live)
          setError(error instanceof Error ? error.message : String(error));
      });
    return () => {
      live = false;
    };
  }, []);
  const test = async (row: ModelConnection, model: string) => {
    setBusy(row.id);
    setError("");
    try {
      upsert(await testConnection(row.id, model));
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy("");
    }
  };
  const remove = async (row: ModelConnection) => {
    setBusy(row.id);
    setError("");
    try {
      await removeConnection(row.id);
      setRows((rows) => rows.filter((r) => r.id !== row.id));
      setRemoving("");
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy("");
    }
  };
  return (
    <SettingsOptionGroup
      title="Model connections"
      description="Connect local compute once and reuse its models across supported harnesses."
      headerAction={
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            setEditing(undefined);
            setOpen(true);
          }}
        >
          Add connection
        </Button>
      }
    >
      {!rows.length && (
        <div className="p-4 text-sm text-muted-foreground">
          Find model servers on this computer and your local network, or add a
          server manually.
        </div>
      )}
      {rows.map((row) => (
        <div key={row.id} className="space-y-2 p-4">
          <div className="flex items-center gap-3">
            <div className="min-w-0 flex-1">
              <p className="font-medium">{row.name}</p>
              <p className="break-all text-xs text-muted-foreground">
                {row.base_url}
              </p>
            </div>
            <Button
              size="sm"
              variant="ghost"
              disabled={!!busy}
              onClick={() => {
                setEditing(row);
                setOpen(true);
              }}
            >
              Edit
            </Button>
            <Button
              size="sm"
              variant="ghost"
              disabled={!!busy}
              onClick={() => setRemoving(row.id)}
            >
              Remove
            </Button>
          </div>
          <p className="text-sm">
            {row.models.length} models ·{" "}
            {row.qualification
              ? `Compatibility tested: ${row.qualification.model}`
              : row.tested_model
                ? `Text response tested: ${row.tested_model}`
                : "Model not tested"}
          </p>
          {row.qualification && (
            <div className="space-y-1 text-xs text-muted-foreground">
              {row.qualification.protocols.map((check) => (
                <p key={check.protocol}>
                  {
                    {
                      chat_completions: "Chat Completions",
                      responses: "Responses",
                      anthropic_messages: "Anthropic Messages",
                    }[check.protocol]
                  }
                  :{" "}
                  {check.tool_round_trip
                    ? "Streaming tools and replies passed"
                    : (check.reason ?? "Not available")}
                </p>
              ))}
              {row.qualification.context_window && (
                <p>
                  Context: {row.qualification.context_window.toLocaleString()}{" "}
                  tokens
                </p>
              )}
            </div>
          )}
          <ConnectionHealth id={row.id} revision={row.base_url} />
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button size="sm" variant="outline" disabled={!!busy}>
                {busy === row.id ? "Working…" : "Test a model"}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent>
              {row.models.map((model) => (
                <DropdownMenuItem
                  key={model}
                  onSelect={() => void test(row, model)}
                >
                  {model}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          <p className="text-xs text-muted-foreground">
            Testing sends several short requests, one at a time, to check tools,
            replies and thinking levels. It can take up to three minutes.
          </p>
          {removing === row.id && (
            <div className="rounded-md border p-3 text-sm">
              Agents using this connection will need another connection before
              starting.
              <div className="mt-2 flex gap-2">
                <Button
                  size="sm"
                  variant="destructive"
                  disabled={!!busy}
                  onClick={() => void remove(row)}
                >
                  Remove connection
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => setRemoving("")}
                >
                  Cancel
                </Button>
              </div>
            </div>
          )}
        </div>
      ))}
      {error && (
        <p role="alert" className="p-4 text-sm text-destructive">
          {error}
        </p>
      )}
      <ConnectModelDialog
        open={open}
        onOpenChange={setOpen}
        existing={editing}
        onConnected={upsert}
      />
    </SettingsOptionGroup>
  );
}
