import { summaryScheduleLabel } from "../lib/summaryScheduleLabel";
import { SummaryRunHistory } from "./SummaryRunHistory";
import * as React from "react";
import { useQuery, useQueryClient, useMutation } from "@tanstack/react-query";
import { useCommunities } from "@/features/communities/useCommunities";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SummaryCollectionCard } from "./SummaryCollectionCard";
import { SummaryReportBrowser, reportContent } from "./SummaryReportBrowser";
import { SummaryEditor } from "./SummaryView";
import { SummaryRunButton } from "./SummaryRunButton";
import {
  fetchSummaryDefinitions,
  fetchSharedSummaryReports,
  saveNamedSummary,
  type SummaryDefinitionRecord,
} from "../lib/namedSummaries";
import type { RelayEvent } from "@/shared/api/types";

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
  const [filter, setFilter] = React.useState<"all" | "paused" | "archived">(
    "all",
  );
  const [search, setSearch] = React.useState("");
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
      <div className="mx-auto w-full max-w-2xl">
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
      </div>
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
        <p className="text-sm text-muted-foreground">
          {summaryScheduleLabel(chosen.config)} ·{" "}
          {chosen.config.channels.length + chosen.config.dms.length}{" "}
          conversations
        </p>
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
          <SummaryReportBrowser
            key={chosen.config.summary_id}
            events={history}
          />
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
  const groups = new Map<string, RelayEvent[]>();
  for (const event of shared) {
    const creator = event.tags.find((t) => t[0] === "owner")?.[1];
    const id = event.tags.find((t) => t[0] === "summary")?.[1] ?? "default";
    const key = `${creator}:${id}`;
    groups.set(key, [...(groups.get(key) ?? []), event]);
  }
  const filtered = all
    .filter(({ config: c }) => {
      const matchesFilter =
        filter === "archived"
          ? c.archived
          : !c.archived && (filter !== "paused" || c.paused);
      const agent =
        agents.data?.find((a) => a.pubkey === c.reporter_pubkey)?.name ?? "";
      return (
        matchesFilter &&
        `${c.name} ${agent}`
          .toLocaleLowerCase()
          .includes(search.trim().toLocaleLowerCase())
      );
    })
    .sort((a, b) => a.config.name.localeCompare(b.config.name));
  return (
    <div className="space-y-7">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="space-y-1">
          <h2 className="text-xl font-semibold tracking-tight">Summaries</h2>
          <p className="text-sm text-muted-foreground">
            Your reports, organized by agent, audience and schedule.
          </p>
        </div>
        <Button onClick={() => setEditor({ draftId: crypto.randomUUID() })}>
          New Summary
        </Button>
      </header>
      {change.isError ? <p role="alert">{change.error.message}</p> : null}
      <section className="space-y-4" aria-label="My Summaries">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h3 className="font-medium">My Summaries</h3>
          <Input
            aria-label="Search Summaries"
            placeholder="Search by name or agent…"
            className="w-full sm:w-64"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <fieldset
          aria-label="Filter Summaries"
          className="flex flex-wrap gap-1"
        >
          {(["all", "paused", "archived"] as const).map((value) => (
            <Button
              key={value}
              size="sm"
              variant={filter === value ? "secondary" : "ghost"}
              aria-pressed={filter === value}
              onClick={() => setFilter(value)}
            >
              {value === "all"
                ? "All"
                : value === "paused"
                  ? "Paused"
                  : "Archived"}
            </Button>
          ))}
        </fieldset>
        <div className="grid gap-4 lg:grid-cols-2">
          {filtered.map((record) => {
            const c = record.config;
            const history = visibleReports.filter((event) =>
              mine(event, c.summary_id),
            );
            return (
              <SummaryCollectionCard
                key={c.summary_id}
                record={record}
                agentName={
                  agents.data?.find((a) => a.pubkey === c.reporter_pubkey)
                    ?.name ?? "Assigned agent"
                }
                reportCount={
                  reports.isError
                    ? null
                    : reports.isLoading
                      ? undefined
                      : history.length
                }
                latestAt={
                  history.length
                    ? Math.max(...history.map((e) => e.created_at))
                    : undefined
                }
                busy={change.isPending}
                onOpen={() => setSelected(c.summary_id)}
                onEdit={() => setEditor({ record, draftId: c.summary_id })}
                onDuplicate={() => {
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
                onPause={() => change.mutate({ record, action: "pause" })}
                onArchive={() => change.mutate({ record, action: "archive" })}
              />
            );
          })}
        </div>
        {!filtered.length ? (
          <p className="rounded-xl border border-dashed p-6 text-sm text-muted-foreground">
            {!all.length
              ? "Create your first Summary to choose an agent and conversations."
              : "No Summaries match this view."}
          </p>
        ) : null}
      </section>
      <section className="space-y-3 border-t pt-6" aria-label="Shared with me">
        <h3 className="font-medium">Shared with me</h3>
        {reports.isError ? (
          <p role="alert">Shared reports could not be refreshed.</p>
        ) : groups.size ? (
          [...groups.entries()].map(([key, events]) => {
            const ordered = [...events].sort(
              (a, b) => b.created_at - a.created_at,
            );
            return (
              <details key={key} className="group rounded-xl border p-4">
                <summary className="cursor-pointer text-sm font-medium">
                  {reportContent(ordered[0])?.title ?? "Shared Summary"}
                  <span className="ml-2 text-xs font-normal text-muted-foreground">
                    {events.length} {events.length === 1 ? "report" : "reports"}
                  </span>
                </summary>
                <div className="mt-5">
                  <SummaryReportBrowser events={ordered} />
                </div>
              </details>
            );
          })
        ) : (
          <p className="text-sm text-muted-foreground">
            Reports shared with you appear here.
          </p>
        )}
      </section>
    </div>
  );
}
