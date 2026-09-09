import { summaryScheduleLabel } from "../lib/summaryScheduleLabel";
import { SummaryRunHistory } from "./SummaryRunHistory";
import * as React from "react";
import { useQuery, useQueryClient, useMutation } from "@tanstack/react-query";
import { useCommunities } from "@/features/communities/useCommunities";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { SummaryEditor } from "./SummaryView";
import { SummaryRunButton } from "./SummaryRunButton";
import {
  fetchSummaryDefinitions,
  fetchSharedSummaryReports,
  saveNamedSummary,
  type SummaryDefinitionRecord,
} from "../lib/namedSummaries";
import type { RelayEvent } from "@/shared/api/types";

function reportContent(event: RelayEvent) {
  try {
    const v = JSON.parse(event.content);
    return typeof v.title === "string" && typeof v.markdown === "string"
      ? (v as { title: string; markdown: string })
      : null;
  } catch {
    return null;
  }
}
function Report({ event }: { event: RelayEvent }) {
  const content = reportContent(event);
  if (!content) return null;
  return (
    <article className="space-y-3 rounded-xl border p-4">
      <h3 className="font-semibold">{content.title}</h3>
      <p className="text-xs text-muted-foreground">
        {new Date(event.created_at * 1000).toLocaleString()}
      </p>
      <Markdown content={content.markdown} />
    </article>
  );
}
export function SummariesView({ currentPubkey }: { currentPubkey?: string }) {
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl ?? "";
  // Remount state on identity/community changes and keep private query caches partitioned.
  return (
    <SummaryCollection
      key={`${relayUrl}:${currentPubkey}`}
      owner={currentPubkey ?? ""}
      relayUrl={relayUrl}
    />
  );
}
function SummaryCollection({
  owner,
  relayUrl,
}: {
  owner: string;
  relayUrl: string;
}) {
  const cache = useQueryClient();
  const agents = useManagedAgentsQuery();
  const [editor, setEditor] = React.useState<{
    record?: SummaryDefinitionRecord;
    draftId: string;
  } | null>(null);
  const [selected, setSelected] = React.useState<string | null>(null);
  const [showArchived, setShowArchived] = React.useState(false);
  const definitions = useQuery({
    queryKey: ["summary-definitions", relayUrl, owner],
    enabled: Boolean(owner && relayUrl),
    queryFn: () => fetchSummaryDefinitions(owner),
  });
  const reports = useQuery({
    queryKey: ["summary-reports", relayUrl, owner],
    enabled: Boolean(owner && relayUrl),
    queryFn: () => fetchSharedSummaryReports(owner),
    refetchInterval: 15000,
    staleTime: 0,
  });
  // Never keep withdrawn recipient content visible after a failed reauthorization/refetch.
  const visibleReports = reports.isError ? [] : (reports.data ?? []);
  const refresh = async () => {
    await Promise.all([definitions.refetch(), reports.refetch()]);
  };
  const change = useMutation({
    mutationFn: ({
      record,
      action,
    }: {
      record: SummaryDefinitionRecord;
      action: "pause" | "archive";
    }) =>
      saveNamedSummary(
        { owner, relayUrl },
        {
          ...record.config,
          ...(action === "pause"
            ? { paused: !record.config.paused }
            : { archived: !record.config.archived }),
        },
        record,
      ),
    onSuccess: refresh,
  });
  React.useEffect(
    () => () => {
      cache.removeQueries({ queryKey: ["summary-reports", relayUrl, owner] });
    },
    [cache, owner, relayUrl],
  );
  if (editor)
    return (
      <SummaryEditor
        key={editor.draftId}
        currentPubkey={owner}
        record={editor.record}
        draftId={editor.draftId}
        onDone={() => {
          setEditor(null);
          void refresh();
        }}
      />
    );
  if (definitions.isLoading) return <p>Loading Summaries…</p>;
  if (definitions.isError)
    return (
      <div role="alert">
        Could not load Summaries.{" "}
        <Button onClick={() => void refresh()}>Retry</Button>
      </div>
    );
  const all = definitions.data ?? [];
  const chosen = all.find((r) => r.config.summary_id === selected);
  const mine = (event: RelayEvent, id: string) =>
    (event.tags.some((t) => t[0] === "owner" && t[1] === owner) &&
      event.tags.some((t) => t[0] === "summary" && t[1] === id)) ||
    (id === "default" &&
      !event.tags.some((t) => t[0] === "summary") &&
      event.tags.some((t) => t[0] === "p" && t[1] === owner));
  if (chosen) {
    const history = visibleReports
      .filter((e) => mine(e, chosen.config.summary_id))
      .sort((a, b) => b.created_at - a.created_at);
    return (
      <div className="space-y-5">
        <Button variant="ghost" onClick={() => setSelected(null)}>
          Back to Summaries
        </Button>
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-semibold">{chosen.config.name}</h2>
          <Button
            variant="outline"
            onClick={() =>
              setEditor({ record: chosen, draftId: chosen.config.summary_id })
            }
          >
            Edit Summary
          </Button>
        </div>
        <SummaryRunButton
          ownerPubkey={owner}
          record={chosen}
          enabled={
            !chosen.config.archived &&
            chosen.config.channels.length + chosen.config.dms.length > 0
          }
          onRequested={() => void reports.refetch()}
        />
        {!chosen.legacy ? (
          <SummaryRunHistory
            scope={{ owner, relayUrl }}
            id={chosen.config.summary_id}
          />
        ) : null}
        {reports.isError ? (
          <p role="alert">
            Report access could not be refreshed.{" "}
            <Button onClick={() => void reports.refetch()}>Retry</Button>
          </p>
        ) : history.length ? (
          history.map((event) => <Report key={event.id} event={event} />)
        ) : (
          <p className="text-muted-foreground">
            No reports yet. Run this Summary when you’re ready.
          </p>
        )}
      </div>
    );
  }
  const shared = visibleReports.filter((e) =>
    e.tags.some((t) => t[0] === "owner" && t[1] !== owner),
  );
  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-lg font-semibold">Summaries</h2>
        <Button onClick={() => setEditor({ draftId: crypto.randomUUID() })}>
          New Summary
        </Button>
      </div>
      <p className="text-sm text-muted-foreground">
        Create separate reports for different people, conversations and
        schedules.
      </p>
      {change.isError ? <p role="alert">{change.error.message}</p> : null}
      <section className="space-y-3">
        <h3 className="font-medium">My Summaries</h3>
        {all
          .filter((r) => showArchived || !r.config.archived)
          .map((record) => {
            const c = record.config;
            return (
              <div
                key={c.summary_id}
                className="space-y-3 rounded-xl border p-4"
              >
                <button
                  type="button"
                  className="text-left font-semibold hover:underline"
                  onClick={() => setSelected(c.summary_id)}
                >
                  {c.name}
                </button>
                <p className="text-sm text-muted-foreground">
                  {agents.data?.find((a) => a.pubkey === c.reporter_pubkey)
                    ?.name ?? "Assigned agent"}{" "}
                  ·{" "}
                  {c.audience.mode === "only_me"
                    ? "Only me"
                    : c.audience.mode === "community"
                      ? "Everyone in this community"
                      : `${c.audience.people.length} selected people`}{" "}
                  · {c.channels.length + c.dms.length} conversations
                </p>
                <p className="text-sm text-muted-foreground">
                  {summaryScheduleLabel(c)}
                </p>
                <div className="flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => setSelected(c.summary_id)}
                  >
                    Open
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setEditor({ record, draftId: c.summary_id })}
                  >
                    Edit
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => {
                      const id = crypto.randomUUID();
                      setEditor({
                        draftId: id,
                        record: {
                          ...record,
                          legacy: false,
                          event: { ...record.event, tags: [] },
                          config: {
                            ...c,
                            summary_id: id,
                            name: `${c.name} copy`,
                            paused: false,
                            archived: false,
                          },
                        },
                      });
                    }}
                  >
                    Duplicate
                  </Button>
                  {!c.archived ? (
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={change.isPending}
                      onClick={() => change.mutate({ record, action: "pause" })}
                    >
                      {c.paused ? "Resume" : "Pause"}
                    </Button>
                  ) : null}
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={change.isPending}
                    onClick={() => change.mutate({ record, action: "archive" })}
                  >
                    {c.archived ? "Restore" : "Archive"}
                  </Button>
                </div>
              </div>
            );
          })}
        {!all.length ? (
          <p className="text-muted-foreground">
            Create your first Summary to choose an agent and conversations.
          </p>
        ) : null}
        {all.some((r) => r.config.archived) ? (
          <Button
            variant="ghost"
            onClick={() => setShowArchived(!showArchived)}
          >
            {showArchived ? "Hide archived" : "Show archived"}
          </Button>
        ) : null}
      </section>
      <section className="space-y-3">
        <h3 className="font-medium">Shared with me</h3>
        {reports.isError ? (
          <p role="alert">Shared reports could not be refreshed.</p>
        ) : shared.length ? (
          shared.map((event) => <Report key={event.id} event={event} />)
        ) : (
          <p className="text-sm text-muted-foreground">
            Reports shared with you appear here.
          </p>
        )}
      </section>
    </div>
  );
}
