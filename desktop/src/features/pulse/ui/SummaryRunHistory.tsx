import { Button } from "@/shared/ui/button";
import { useMutation, useQuery } from "@tanstack/react-query";
import {
  summaryOperation,
  type SummaryScope,
  type SummaryRunReceipt,
} from "../lib/namedSummaries";
export function SummaryRunHistory({
  scope,
  id,
}: {
  scope: SummaryScope;
  id: string;
}) {
  const query = useQuery({
    queryKey: ["summary-runs", scope.relayUrl, scope.owner, id],
    queryFn: () =>
      summaryOperation<{ runs: SummaryRunReceipt[] }>(scope, {
        summary_operation: "history",
        summary_id: id,
      }),
    refetchInterval: 5000,
  });
  const cancel = useMutation({
    mutationFn: (run: string) =>
      summaryOperation(scope, {
        summary_operation: "progress",
        summary_id: id,
        run_id: run,
        state: "failed",
        message: "Cancelled by owner; cursors unchanged",
      }),
    onSuccess: () => query.refetch(),
  });
  if (query.isError)
    return <p role="alert">Run history could not be refreshed.</p>;
  if (!query.data?.runs.length) return null;
  return (
    <details className="rounded-xl border p-3">
      <summary className="cursor-pointer text-sm font-medium">
        Run history · {query.data.runs[0].state.replaceAll("_", " ")}
      </summary>
      {cancel.isError ? <p role="alert">{cancel.error.message}</p> : null}
      <ul className="mt-3 space-y-2">
        {query.data.runs.map((run) => (
          <li key={run.run_id} className="text-sm">
            <span>
              {new Date(run.started_at).toLocaleString()} ·{" "}
              {run.state.replaceAll("_", " ")}
            </span>
            {["running", "queued", "needs_approval"].includes(run.state) &&
            new Date(run.expires_at).getTime() > Date.now() ? (
              <Button
                size="sm"
                variant="ghost"
                disabled={cancel.isPending}
                onClick={() => cancel.mutate(run.run_id)}
              >
                Cancel run
              </Button>
            ) : null}
            {run.error ? <p className="text-destructive">{run.error}</p> : null}
            {["running", "queued", "needs_approval"].includes(run.state) &&
            new Date(run.expires_at).getTime() < Date.now() ? (
              <p className="text-muted-foreground">
                Execution lease expired. The next request starts a new run;
                cursors remain unchanged.
              </p>
            ) : null}
          </li>
        ))}
      </ul>
    </details>
  );
}
