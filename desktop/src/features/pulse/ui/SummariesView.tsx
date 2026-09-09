import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { SummaryReportReader } from "./SummaryReportReader";
import { summaryScheduleLabel } from "../lib/summaryScheduleLabel";
import { SummaryReportLibrary } from "./SummaryReportLibrary";
import { SummaryRunHistory } from "./SummaryRunHistory";
import * as React from "react";
import { useQuery, useQueryClient, useMutation } from "@tanstack/react-query";
import { useCommunities } from "@/features/communities/useCommunities";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SummaryCollectionCard } from "./SummaryCollectionCard";
import {
  fetchSummaryReportPage,
  type SummaryReportItem,
  type SummaryOverview,
} from "../lib/summaryReports";
import { SummaryEditor } from "./SummaryView";
import { SummaryRunButton } from "./SummaryRunButton";
import {
  fetchSummaryDefinitions,
  summaryOperation,
  saveNamedSummary,
  type SummaryDefinitionRecord,
} from "../lib/namedSummaries";

const SUMMARY_LINK_KEYS = ["summaryReport"] as const;
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
  const { values: linked, applyPatch } =
    useHistorySearchState(SUMMARY_LINK_KEYS);
  const linkQuery = useQuery({
    queryKey: ["summary-link", relayUrl, owner, linked.summaryReport],
    enabled: Boolean(owner && relayUrl && linked.summaryReport),
    retry: false,
    gcTime: 0,
    queryFn: () =>
      summaryOperation<{ report: SummaryReportItem }>(
        { owner, relayUrl },
        { summary_operation: "reports_get", event_ids: [linked.summaryReport] },
      ),
  });
  const agents = useManagedAgentsQuery();
  const [editor, setEditor] = React.useState<{
    record?: SummaryDefinitionRecord;
    draftId: string;
  } | null>(null);
  const [selected, setSelected] = React.useState<string | null>(null);
  const [filter, setFilter] = React.useState<"all" | "paused" | "archived">(
    "all",
  );
  const [library, setLibrary] = React.useState<{
    archived: boolean;
    summaryId?: string;
    summaryOwner?: string;
  } | null>(null);
  const [search, setSearch] = React.useState("");
  const capabilities = useQuery({
    queryKey: ["summary-capabilities", relayUrl, owner],
    enabled: Boolean(owner && relayUrl),
    queryFn: () =>
      summaryOperation<{
        summary_protocol?: number;
        summary_hardening?: number;
        summary_report_archive?: number;
      }>({ owner, relayUrl }, { summary_operation: "capabilities" }),
    retry: false,
  });
  const supported =
    capabilities.data?.summary_protocol === 2 &&
    capabilities.data?.summary_hardening === 1 &&
    capabilities.data?.summary_report_archive === 1;
  const overview = useQuery({
    queryKey: ["summary-overview", relayUrl, owner],
    enabled: supported && !library && !editor && !selected,
    queryFn: () =>
      summaryOperation<{ summaries: SummaryOverview[] }>(
        { owner, relayUrl },
        { summary_operation: "overview" },
      ),
    refetchInterval: 15000,
  });
  const definitions = useQuery({
    queryKey: ["summary-definitions", relayUrl, owner],
    enabled: Boolean(owner && relayUrl) && supported,
    queryFn: () => fetchSummaryDefinitions(owner, relayUrl),
  });
  const reports = useQuery({
    queryKey: ["summary-reports", relayUrl, owner],
    enabled:
      Boolean(owner && relayUrl) &&
      supported &&
      !library &&
      !editor &&
      !selected,
    queryFn: async () =>
      (await fetchSummaryReportPage({ owner, relayUrl }, { archived: false }))
        .reports,
    refetchInterval: 15000,
    staleTime: 0,
  });
  // Never keep withdrawn recipient content visible after a failed reauthorization/refetch.
  const visibleReports = reports.isError ? [] : (reports.data ?? []);
  const refresh = async () => {
    await Promise.all([
      definitions.refetch(),
      reports.refetch(),
      overview.refetch(),
    ]);
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
  if (capabilities.isLoading) return <p>Checking Summary support…</p>;
  if (capabilities.isError)
    return (
      <div role="alert">
        Could not reach the relay to check Summary support.{" "}
        <Button onClick={() => void capabilities.refetch()}>Retry</Button>
      </div>
    );
  if (!supported)
    return (
      <div role="alert">
        This relay needs an update to use this version of Summaries. Your saved
        reports remain stored on the relay.{" "}
        <Button onClick={() => void capabilities.refetch()}>Check again</Button>
      </div>
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
  if (linked.summaryReport)
    return (
      <div className="space-y-4">
        <Button
          variant="ghost"
          onClick={() => applyPatch({ summaryReport: null })}
        >
          Back to Summaries
        </Button>
        {linkQuery.isError ? (
          <p role="alert">
            This report is unavailable or you no longer have access.
          </p>
        ) : linkQuery.data ? (
          <SummaryReportReader
            scope={{ owner, relayUrl }}
            item={linkQuery.data.report}
            onClose={() => applyPatch({ summaryReport: null })}
          />
        ) : (
          <p>Loading report…</p>
        )}
      </div>
    );
  const all = definitions.data ?? [];
  const chosen = all.find((r) => r.config.summary_id === selected);
  if (library)
    return (
      <div className="space-y-5">
        <Button variant="ghost" onClick={() => setLibrary(null)}>
          Back to Summaries
        </Button>
        <h2 className="text-xl font-semibold">
          {library.archived ? "Summary archive" : "Report library"}
        </h2>
        {library.summaryId && (
          <p className="text-sm text-muted-foreground">
            {all.find((r) => r.config.summary_id === library.summaryId)?.config
              .name ?? "Selected Summary"}
          </p>
        )}
        <div className="flex gap-2">
          <Button
            aria-pressed={!library.archived}
            variant={!library.archived ? "default" : "ghost"}
            onClick={() => setLibrary({ ...library, archived: false })}
          >
            Current reports
          </Button>
          <Button
            aria-pressed={library.archived}
            variant={library.archived ? "default" : "ghost"}
            onClick={() => setLibrary({ ...library, archived: true })}
          >
            Archived reports
          </Button>
        </div>
        {change.isError && <p role="alert">{change.error.message}</p>}
        {library.archived && !library.summaryId && (
          <details
            className="space-y-3 rounded-xl border p-4"
            aria-label="Archived Summary setups"
          >
            <summary className="cursor-pointer font-semibold">
              Archived Summary setups (
              {all.filter((r) => r.config.archived).length})
            </summary>
            <p className="text-sm text-muted-foreground">
              These schedules are stopped. Their reports remain available
              separately.
            </p>
            <Input
              aria-label="Search archived Summary setups"
              placeholder="Find an archived setup by name or agent…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
            {all
              .filter(
                (r) =>
                  r.config.archived &&
                  `${r.config.name} ${agents.data?.find((a) => a.pubkey === r.config.reporter_pubkey)?.name ?? ""}`
                    .toLocaleLowerCase()
                    .includes(search.toLocaleLowerCase()),
              )
              .map((r) => (
                <div
                  key={r.config.summary_id}
                  className="flex items-center gap-3 rounded-lg border p-3"
                >
                  <div className="min-w-0 flex-1">
                    <p className="font-medium">{r.config.name}</p>
                    <p className="text-sm text-muted-foreground">
                      {agents.data?.find(
                        (a) => a.pubkey === r.config.reporter_pubkey,
                      )?.name ?? "Assigned agent"}
                    </p>
                  </div>
                  <Button
                    variant="ghost"
                    onClick={() => {
                      setLibrary(null);
                      setSelected(r.config.summary_id);
                    }}
                  >
                    Open
                  </Button>
                  <Button
                    variant="outline"
                    disabled={change.isPending}
                    onClick={() =>
                      change.mutate({ record: r, action: "archive" })
                    }
                  >
                    Restore setup
                  </Button>
                </div>
              ))}
            {!all.some((r) => r.config.archived) && (
              <p className="text-sm text-muted-foreground">
                No archived setups.
              </p>
            )}
          </details>
        )}
        <SummaryReportLibrary
          key={`${library.summaryOwner}:${library.summaryId}`}
          scope={{ owner, relayUrl }}
          definitions={all}
          agents={agents.data ?? []}
          {...library}
        />
      </div>
    );
  if (chosen) {
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
        <Button
          variant="outline"
          onClick={() =>
            setLibrary({ archived: false, summaryId: chosen.config.summary_id })
          }
        >
          Manage reports
        </Button>
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
            record={chosen}
            scope={{ owner, relayUrl }}
            id={chosen.config.summary_id}
          />
        ) : null}
        <SummaryReportLibrary
          key={chosen.config.summary_id}
          scope={{ owner, relayUrl }}
          definitions={all}
          agents={agents.data ?? []}
          summaryId={chosen.config.summary_id}
          openLatest
        />
      </div>
    );
  }
  const shared = visibleReports.filter((e) => e.owner !== owner);
  const groups = new Map<string, SummaryReportItem[]>();
  for (const event of shared) {
    const key = `${event.owner}:${event.summary_id}`;
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
      <nav aria-label="Summary organization" className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          onClick={() => setLibrary({ archived: false })}
        >
          Browse all reports
        </Button>
        <Button
          variant="outline"
          onClick={() => {
            setFilter("archived");
            setLibrary({ archived: true });
          }}
        >
          Summary archive
        </Button>
      </nav>
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
            const stats = overview.data?.summaries.find(
              (s) => s.summary_id === c.summary_id,
            );
            return (
              <SummaryCollectionCard
                key={c.summary_id}
                record={record}
                agentName={
                  agents.data?.find((a) => a.pubkey === c.reporter_pubkey)
                    ?.name ?? "Assigned agent"
                }
                nextRunAt={stats?.next_run_at}
                reportCount={overview.isError ? null : stats?.report_count}
                latestAt={stats?.latest_at ?? undefined}
                runState={stats?.state ?? undefined}
                runError={stats?.error ?? undefined}
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
        <h3 className="font-medium">Recently shared with me</h3>
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
                  {ordered[0].summary_name} · {ordered[0].owner_name}
                  <span className="ml-2 text-xs font-normal text-muted-foreground">
                    {events.length} {events.length === 1 ? "report" : "reports"}
                  </span>
                </summary>
                <div className="mt-5">
                  <Button
                    variant="outline"
                    onClick={() =>
                      setLibrary({
                        archived: false,
                        summaryId: ordered[0].summary_id,
                        summaryOwner: ordered[0].owner,
                      })
                    }
                  >
                    Browse shared reports
                  </Button>
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
