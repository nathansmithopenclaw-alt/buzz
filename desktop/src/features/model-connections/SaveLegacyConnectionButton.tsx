import { useState } from "react";
import { Button } from "@/shared/ui/button";
import { saveConnection } from "./api";
export function SaveLegacyConnectionButton({
  url,
  apiKey,
  disabled,
  onSaved,
}: {
  url: string;
  apiKey?: string;
  disabled: boolean;
  onSaved: (id: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  return (
    <div className="space-y-1">
      <Button
        type="button"
        variant="outline"
        disabled={disabled || busy || !url.trim()}
        onClick={async () => {
          setBusy(true);
          setError("");
          try {
            const name = new URL(url.includes("://") ? url : `http://${url}`)
              .hostname;
            const row = await saveConnection({
              name,
              base_url: url,
              api_key: apiKey,
            });
            onSaved(row.id);
          } catch (error) {
            setError(error instanceof Error ? error.message : String(error));
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy ? "Saving connection…" : "Save as reusable connection"}
      </Button>
      <p className="text-xs text-muted-foreground">
        Reuse this server in other agents without entering its address again.
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}
