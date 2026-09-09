import { useCallback, useEffect, useRef, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Switch } from "@/shared/ui/switch";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  discoverServers,
  saveConnection,
  type DiscoveredServer,
  type ModelConnection,
} from "./api";

export function ConnectModelDialog({
  open,
  onOpenChange,
  onConnected,
  existing,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConnected: (connection: ModelConnection) => void;
  existing?: ModelConnection;
}) {
  const [privateNetwork, setPrivateNetwork] = useState(false);
  const [servers, setServers] = useState<DiscoveredServer[]>([]);
  const [searching, setSearching] = useState(false);
  const [saving, setSaving] = useState(false);
  const [manual, setManual] = useState(false);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [key, setKey] = useState("");
  const [clearKey, setClearKey] = useState(false);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const generation = useRef(0);
  const search = useCallback(async () => {
    const token = ++generation.current;
    setSearching(true);
    setError("");
    try {
      const result = await discoverServers(privateNetwork);
      if (token !== generation.current) return;
      setServers(result.servers);
      setNotice(result.notice);
    } catch (error) {
      if (token === generation.current) setError(String(error));
    } finally {
      if (token === generation.current) setSearching(false);
    }
  }, [privateNetwork]);
  useEffect(() => {
    if (!open) return;
    setManual(Boolean(existing));
    setName(existing?.name ?? "");
    setUrl(existing?.base_url ?? "");
    setKey("");
    setClearKey(false);
    setError("");
    if (!existing) void search();
    return () => {
      generation.current++;
    };
  }, [open, existing, search]);
  const connect = async (server?: DiscoveredServer) => {
    if (server?.needs_key) {
      setName(server.name);
      setUrl(server.base_url);
      setManual(true);
      return;
    }
    setSaving(true);
    setError("");
    try {
      const row = await saveConnection({
        id: existing?.id,
        name: server?.name ?? name,
        base_url: server?.base_url ?? url,
        api_key: clearKey ? "" : key || undefined,
      });
      onConnected(row);
      onOpenChange(false);
    } catch (error) {
      setError(String(error));
    } finally {
      setSaving(false);
    }
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!saving) onOpenChange(value);
      }}
    >
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>
            {existing ? "Edit model connection" : "Connect a model server"}
          </DialogTitle>
          <DialogDescription>
            Connect once, then use its models with supported agent harnesses.
            Servers are checked from this computer.
          </DialogDescription>
        </DialogHeader>
        {!manual && (
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="font-medium">This computer and local network</h3>
              <Button
                variant="outline"
                size="sm"
                disabled={searching || saving}
                onClick={() => void search()}
              >
                Search again
              </Button>
            </div>
            <div className="flex items-center gap-2 text-sm">
              <Switch
                id="model-private-network"
                checked={privateNetwork}
                disabled={searching || saving}
                onCheckedChange={setPrivateNetwork}
              />
              <label htmlFor="model-private-network">
                Include private network (Tailscale)
              </label>
            </div>
            {searching && (
              <p role="status" className="text-sm text-muted-foreground">
                Finding model servers… This can take up to 30 seconds.
              </p>
            )}
            {servers.map((server) => (
              <div
                key={server.base_url}
                className="flex items-center gap-3 rounded-lg border p-3"
              >
                <div className="min-w-0 flex-1">
                  <div className="font-medium">{server.name}</div>
                  <div className="break-all text-xs text-muted-foreground">
                    {server.base_url}
                  </div>
                  <p className="text-sm">
                    {server.needs_key
                      ? "Authentication required · service not yet verified"
                      : server.models.slice(0, 3).join(", ")}
                    {server.models.length > 3
                      ? ` and ${server.models.length - 3} more`
                      : ""}
                  </p>
                </div>
                <Button disabled={saving} onClick={() => void connect(server)}>
                  {server.needs_key ? "Enter key" : "Connect"}
                </Button>
              </div>
            ))}
            {!searching && servers.length === 0 && (
              <p className="text-sm">
                No model servers found. Make sure your server is running and
                allows connections from this computer.
              </p>
            )}
            {notice && (
              <p className="text-xs text-muted-foreground">{notice}</p>
            )}
            <Button
              variant="ghost"
              disabled={saving}
              onClick={() => setManual(true)}
            >
              Don’t see your server? Add manually
            </Button>
          </div>
        )}
        {manual && (
          <form
            className="space-y-4"
            onSubmit={(event) => {
              event.preventDefault();
              void connect();
            }}
          >
            <div className="space-y-1">
              <label htmlFor="model-connection-name">Connection name</label>
              <Input
                id="model-connection-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Spark 1"
                required
                maxLength={100}
              />
            </div>
            <div className="space-y-1">
              <label htmlFor="model-connection-url">Server address</label>
              <Input
                id="model-connection-url"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="computer.local:8000"
                required
                autoComplete="off"
              />
              <p className="text-xs text-muted-foreground">
                A hostname or full API address. Works with OpenAI-compatible
                servers, including private-network addresses.
              </p>
            </div>
            <div className="space-y-1">
              <label htmlFor="model-connection-key">API key (optional)</label>
              <Input
                id="model-connection-key"
                disabled={clearKey}
                type="password"
                value={key}
                onChange={(e) => setKey(e.target.value)}
                placeholder={
                  existing?.has_key
                    ? "Keep saved key"
                    : "Leave blank if no key is needed"
                }
                autoComplete="off"
              />
            </div>
            {existing?.has_key && (
              <div className="flex items-center gap-2 text-sm">
                <Switch
                  id="remove-model-key"
                  checked={clearKey}
                  onCheckedChange={setClearKey}
                />
                <label htmlFor="remove-model-key">Remove saved API key</label>
              </div>
            )}
            {existing && (
              <p className="text-xs text-muted-foreground">
                Changes apply to agents using this connection when they next
                start.
              </p>
            )}
            <div className="flex justify-end gap-2">
              {!existing && (
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => setManual(false)}
                >
                  Back to discovery
                </Button>
              )}
              <Button
                type="submit"
                disabled={saving || !name.trim() || !url.trim()}
              >
                {saving ? "Connecting…" : "Save connection"}
              </Button>
            </div>
          </form>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}
