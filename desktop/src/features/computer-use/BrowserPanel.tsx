import { Globe } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { invokeTauri } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/shared/ui/dialog";

type Status = {
  enabled: boolean;
  dependencyReady: boolean;
  nativeEnabled: boolean;
  nativeReady: boolean;
  detail: string;
};
type Session = {
  id: string;
  owner: string;
  label?: string;
  viewerUrl: string;
  created: string;
};

/** Human inspection of the same local browser sessions exposed to agents over MCP. */
export function BrowserPanel() {
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<Status | null>(null);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const refresh = useCallback(async () => {
    const [nextStatus, nextSessions] = await Promise.all([
      invokeTauri<Status>("computer_use_status"),
      invokeTauri<Session[]>("list_computer_browser_sessions"),
    ]);
    setStatus(nextStatus);
    setSessions(nextSessions);
    setSelected((current) =>
      nextSessions.some((s) => s.id === current)
        ? current
        : (nextSessions[0]?.id ?? null),
    );
  }, []);
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        await refresh();
      } catch (reason) {
        if (!cancelled) setError(String(reason));
      } finally {
        if (!cancelled) timer = setTimeout(poll, 2000);
      }
    };
    void poll();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [open, refresh]);
  const toggle = async () => {
    if (!status) return;
    setSaving(true);
    setError(null);
    try {
      await invokeTauri("set_computer_use_enabled", {
        enabled: !status.enabled,
      });
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };
  const toggleNative = async () => {
    if (!status) return;
    setSaving(true);
    setError(null);
    try {
      await invokeTauri("set_computer_native_enabled", {
        enabled: !status.nativeEnabled,
      });
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };
  const session = sessions.find((s) => s.id === selected);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button
          variant="ghost"
          className="w-full justify-start gap-2"
          aria-label="Open agent browsers"
        >
          <Globe className="size-4" />
          <span className="group-data-[collapsible=icon]:hidden">Browser</span>
        </Button>
      </DialogTrigger>
      <DialogContent className="flex h-[85vh] max-w-[90vw] flex-col gap-3">
        <DialogHeader>
          <DialogTitle>Agent browsers</DialogTitle>
          <DialogDescription>
            Inspect an agent’s local browser and take control of the same page.
          </DialogDescription>
        </DialogHeader>
        <div className="flex items-center gap-3">
          <p className="flex-1 text-sm text-muted-foreground">
            {status?.detail ?? "Checking browser availability…"}
          </p>
          <Button
            variant="outline"
            disabled={!status?.dependencyReady || saving}
            onClick={() => void toggle()}
          >
            {status?.enabled
              ? "Disable for new agent starts"
              : "Enable browser tools"}
          </Button>
        </div>
        <div className="flex items-center gap-3">
          <p className="flex-1 text-sm text-muted-foreground">
            Native app control uses one shared desktop lease. macOS
            Accessibility and Screen Recording permission must be granted to
            Buzz. Disable and re-enable to reset the driver after a failed
            action.
          </p>
          <Button
            variant="outline"
            disabled={!status?.nativeReady || !status?.enabled || saving}
            onClick={() => void toggleNative()}
          >
            {status?.nativeEnabled
              ? "Disable native control"
              : "Enable native control"}
          </Button>
        </div>
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
        <nav
          className="flex gap-2 overflow-auto"
          aria-label="Agent browser connections"
        >
          {sessions.map((s, index) => (
            <Button
              key={s.id}
              variant={selected === s.id ? "secondary" : "ghost"}
              title={s.owner}
              onClick={() => setSelected(s.id)}
            >
              {s.label || `Connection ${index + 1}`}
            </Button>
          ))}
        </nav>
        {session ? (
          <iframe
            key={session.id}
            title="Live agent browser"
            src={session.viewerUrl}
            sandbox="allow-scripts allow-same-origin allow-forms"
            referrerPolicy="no-referrer"
            className="min-h-0 w-full flex-1 rounded-md border"
          />
        ) : (
          <div className="flex flex-1 items-center justify-center rounded-md border text-sm text-muted-foreground">
            No active browser connections. Start or restart an agent after
            enabling the tools.
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
