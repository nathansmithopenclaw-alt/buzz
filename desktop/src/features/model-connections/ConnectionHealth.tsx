import { useQuery } from "@tanstack/react-query";
import { invokeTauri } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
export function ConnectionHealth({
  id,
  revision,
}: {
  id: string;
  revision: string;
}) {
  const query = useQuery({
    queryKey: ["model-connection-health", id, revision],
    queryFn: () => invokeTauri<string[]>("check_model_connection", { id }),
    retry: false,
    staleTime: 30_000,
  });
  const message = query.isFetching
    ? "Checking availability…"
    : query.error
      ? `Unavailable: ${String(query.error)}`
      : `Available · ${query.data?.length ?? 0} models`;
  return (
    <div
      className="flex items-center gap-2 text-xs text-muted-foreground"
      role="status"
    >
      <span>{message}</span>
      {query.error && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => void query.refetch()}
        >
          Retry
        </Button>
      )}
    </div>
  );
}
