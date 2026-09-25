import { ConnectionHealth } from "./ConnectionHealth";
import { useEffect, useState } from "react";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { ConnectModelDialog } from "./ConnectModelDialog";
import { listConnections, type ModelConnection } from "./api";
export function ModelConnectionPicker({
  value,
  onChange,
  disabled = false,
}: {
  value: string;
  onChange: (id: string) => void;
  disabled?: boolean;
}) {
  const [rows, setRows] = useState<ModelConnection[]>([]);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState("");
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
  const selected = rows.find((row) => row.id === value);
  return (
    <div className="space-y-2">
      <span className="text-sm font-medium">Model connection</span>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant="outline"
            disabled={disabled}
            className="w-full justify-between"
            aria-label="Model connection"
          >
            {selected?.name ??
              (value
                ? "Connection unavailable — choose another"
                : "Choose a connection")}
            <span aria-hidden>⌄</span>
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent className="max-w-sm">
          {rows.map((row) => (
            <DropdownMenuItem key={row.id} onSelect={() => onChange(row.id)}>
              {row.name}
            </DropdownMenuItem>
          ))}
          <DropdownMenuItem onSelect={() => setOpen(true)}>
            Connect another model server…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      {selected && (
        <ConnectionHealth id={selected.id} revision={selected.base_url} />
      )}
      {selected && (
        <p className="text-xs text-muted-foreground">
          {selected.models.length} models ·{" "}
          {selected.qualification && selected.tested_model
            ? "Tools and replies tested"
            : selected.tested_model
              ? "Text response tested"
              : "Model not tested"}
        </p>
      )}
      {!rows.length && (
        <Button
          variant="ghost"
          type="button"
          disabled={disabled}
          onClick={() => setOpen(true)}
        >
          Find model servers
        </Button>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <ConnectModelDialog
        open={open}
        onOpenChange={setOpen}
        onConnected={(row) => {
          setRows((old) => [...old.filter((r) => r.id !== row.id), row]);
          onChange(row.id);
        }}
      />
    </div>
  );
}
