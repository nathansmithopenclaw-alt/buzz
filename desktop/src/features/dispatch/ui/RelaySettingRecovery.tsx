import * as React from "react";
import { Button } from "@/shared/ui/button";
import {
  pendingRelayAuthorization,
  saveRelayAuthorization,
} from "../relayAutomation";
import type { DispatchContext } from "../types";
import { useDispatchJournalVersion } from "./useDispatchJournalVersion";

/** Recovery stays available even when a partially saved setting now looks paused. */
export function RelaySettingRecovery({
  stream,
  setting,
  refresh,
  onRecovered,
}: {
  stream: string;
  setting: string;
  refresh: () => Promise<DispatchContext>;
  onRecovered: () => void;
}) {
  const version = useDispatchJournalVersion();
  const [pending, setPending] =
    React.useState<Awaited<ReturnType<typeof pendingRelayAuthorization>>>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  React.useEffect(() => {
    void version; // A journal write triggers a fresh read even if the setting key is unchanged.
    let active = true;
    void pendingRelayAuthorization(stream, setting)
      .then((value) => {
        if (active) setPending(value);
      })
      .catch((cause) => {
        if (active) setError(String(cause));
      });
    return () => {
      active = false;
    };
  }, [stream, setting, version]);
  if (!pending && !error) return null;
  return (
    <div
      className="space-y-2 rounded border border-border p-3 text-sm"
      role="status"
    >
      <p>
        A setting change still needs confirmation. Retry sends the saved choice.
      </p>
      {error ? (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      ) : null}
      <Button
        size="sm"
        variant="outline"
        disabled={busy || !pending}
        onClick={() => {
          if (!pending) return;
          setBusy(true);
          setError(null);
          void saveRelayAuthorization({ ...pending, refresh })
            .then(async () => {
              await refresh();
              onRecovered();
            })
            .catch((cause) => setError(String(cause)))
            .finally(() => setBusy(false));
        }}
      >
        Retry saved setting
      </Button>
    </div>
  );
}
