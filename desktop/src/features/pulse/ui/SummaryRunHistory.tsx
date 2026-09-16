import { useNavigate } from "@tanstack/react-router";
import { openDm } from "@/shared/api/tauriChannels";
import { runNamedSummary } from "../lib/runSummary";
import type { SummaryDefinitionRecord } from "../lib/namedSummaries";
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
  record,
}: {
  scope: SummaryScope;
  id: string;
  record?: SummaryDefinitionRecord;
}) {
  const navigate = useNavigate();
  const conversation = useMutation({
    mutationFn: (reporter: string) =>
      openDm({
        pubkeys: [reporter],
        expectedRelayUrl: scope.relayUrl,
        expectedSignerPubkey: scope.owner,
      }),
    onSuccess: (channel) => {
      void navigate({
        to: "/channels/$channelId",
        params: { channelId: channel.id },
      });
    },
  });
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
  const retry = useMutation({
    mutationFn: async (run: string) => {
      if (!record) return;
      await cancel.mutateAsync(run);
      await runNamedSummary(scope.owner, scope.relayUrl, record);
    },
    onSettled: () => query.refetch(),
  });
  if (query.isError)
    return <p role="alert">Run history could not be refreshed.</p>;
  if (!query.data?.runs.length) return null;
  return (
    <details className="rounded-xl border p-3">
      <summary className="cursor-pointer text-sm font-medium">
        Run history · {query.data.runs[0].state.replaceAll("_", " ")}
      </summary>
      {conversation.isError ? (
        <p role="alert">{conversation.error.message}</p>
      ) : null}
      {retry.isError ? <p role="alert">{retry.error.message}</p> : null}
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
                disabled={cancel.isPending || retry.isPending}
                onClick={() => cancel.mutate(run.run_id)}
              >
                Cancel run
              </Button>
            ) : null}
            {run.error ? <p className="text-destructive">{run.error}</p> : null}
            {record && run.state === "needs_approval" ? (
              <div className="space-y-2">
                <p className="text-muted-foreground">
                  Resolve any approval in the agent conversation first. If this
                  run cannot continue, cancel and retry starts a new request
                  without moving reading progress.
                </p>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={conversation.isPending}
                  onClick={() => conversation.mutate(run.reporter_pubkey)}
                >
                  View agent conversation
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={cancel.isPending || retry.isPending}
                  onClick={() => retry.mutate(run.run_id)}
                >
                  Cancel and retry
                </Button>
              </div>
            ) : null}
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
