export function DispatchSyncNotice({ error }: { error: Error | null }) {
  return (
    <p
      role="status"
      className="shrink-0 border-b border-border px-5 py-2 text-xs text-muted-foreground"
    >
      Showing saved issues while Dispatch checks for updates. Editing becomes
      available when syncing finishes.
      {error ? ` Could not refresh: ${error.message}` : ""}
    </p>
  );
}
