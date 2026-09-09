import { runNamedSummary } from "../lib/runSummary";
import * as React from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogCancel,
} from "@/shared/ui/alert-dialog";
import { SummaryFilterSelect } from "./SummaryFilterSelect";
import {
  summaryOperation,
  type SummaryScope,
  type SummaryDefinitionRecord,
} from "../lib/namedSummaries";
import {
  fetchSummaryReportPage,
  type SummaryReportItem,
  type ReportCursor,
} from "../lib/summaryReports";
import { SummaryReportReader } from "./SummaryReportReader";

const message = (error: unknown) =>
  error instanceof Error ? error.message : "Please refresh and try again.";
const summaryKey = (owner: string, id: string) => `${owner}:${id}`;
type LibraryDraft = {
  query: string;
  summary: string;
  reporter: string;
  owner: string;
  since: string;
  until: string;
  positions: {
    current: (ReportCursor | null)[];
    archived: (ReportCursor | null)[];
  };
};
// Session-only navigation state, scoped to the reader and relay. No report bodies.
const drafts = new Map<string, LibraryDraft>();
export function SummaryReportLibrary({
  scope,
  definitions,
  agents,
  archived = false,
  summaryId,
  summaryOwner,
  openLatest = false,
}: {
  scope: SummaryScope;
  definitions: SummaryDefinitionRecord[];
  agents: { pubkey: string; name: string }[];
  archived?: boolean;
  summaryId?: string;
  summaryOwner?: string;
  openLatest?: boolean;
}) {
  const cache = useQueryClient(),
    labelId = React.useId();
  const draftKey = JSON.stringify([
    scope.relayUrl,
    scope.owner,
    summaryOwner,
    summaryId,
  ]);
  const saved = React.useRef(drafts.get(draftKey)).current;
  const [query, setQuery] = React.useState(saved?.query ?? ""),
    [debounced, setDebounced] = React.useState(saved?.query ?? "");
  const [summary, setSummary] = React.useState(
    saved?.summary ??
      (summaryId ? summaryKey(summaryOwner ?? scope.owner, summaryId) : "all"),
  );
  const [reporter, setReporter] = React.useState(saved?.reporter ?? "all"),
    [owner, setOwner] = React.useState(saved?.owner ?? "all");
  const [since, setSince] = React.useState(saved?.since ?? ""),
    [until, setUntil] = React.useState(saved?.until ?? "");
  const [positions, setPositions] = React.useState<LibraryDraft["positions"]>(
    saved?.positions ?? { current: [null], archived: [null] },
  );
  const mode = archived ? "archived" : "current";
  const cursors = positions[mode];
  const setCursors = (value: React.SetStateAction<(ReportCursor | null)[]>) =>
    setPositions((previous) => ({
      ...previous,
      [mode]: typeof value === "function" ? value(previous[mode]) : value,
    }));
  React.useEffect(() => {
    drafts.set(draftKey, {
      query,
      summary,
      reporter,
      owner,
      since,
      until,
      positions,
    });
    const oldest = drafts.keys().next().value;
    if (drafts.size > 32 && oldest !== undefined) drafts.delete(oldest);
  }, [draftKey, query, summary, reporter, owner, since, until, positions]);
  const [selected, setSelected] = React.useState<string[]>([]),
    [opened, setOpened] = React.useState<SummaryReportItem | null>(null);
  const [deleting, setDeleting] = React.useState<SummaryReportItem[]>([]);
  const [known, setKnown] = React.useState<SummaryReportItem[]>([]);
  const lastOpened = React.useRef<string | null>(null);
  const initialOpen = React.useRef(false);
  React.useEffect(() => {
    const timer = setTimeout(() => setDebounced(query.trim()), 250);
    return () => clearTimeout(timer);
  }, [query]);
  const reset = () => {
    setPositions({ current: [null], archived: [null] });
    setSelected([]);
    setOpened(null);
  };
  const previousArchiveMode = React.useRef(archived);
  React.useEffect(() => {
    if (previousArchiveMode.current !== archived) {
      previousArchiveMode.current = archived;
      setSelected([]);
      setOpened(null);
    }
  }, [archived]);
  const invalidDates = Boolean(since && until && since > until);
  const filter = {
    archived,
    query: debounced || undefined,
    owner:
      summary !== "all"
        ? summary.split(":")[0]
        : owner !== "all"
          ? owner
          : undefined,
    summary_id: summary !== "all" ? summary.split(":")[1] : undefined,
    reporter: reporter !== "all" ? reporter : undefined,
    since: since
      ? Math.floor(new Date(`${since}T00:00:00`).getTime() / 1000)
      : undefined,
    until: until
      ? Math.floor(new Date(`${until}T23:59:59`).getTime() / 1000)
      : undefined,
    ...cursors[cursors.length - 1],
  };
  const page = useQuery({
    queryKey: ["summary-library", scope.relayUrl, scope.owner, filter],
    queryFn: () => fetchSummaryReportPage(scope, filter),
    enabled: !invalidDates,
    refetchInterval: 15000,
    staleTime: 0,
    gcTime: 0,
    retry: false,
  });
  const reports =
    page.isError || invalidDates ? [] : (page.data?.reports ?? []);
  React.useEffect(() => {
    if (openLatest && !initialOpen.current && page.data) {
      initialOpen.current = true;
      const first = page.data.reports[0];
      if (first) {
        lastOpened.current = first.id;
        setOpened(first);
      }
    }
  }, [openLatest, page.data]);
  React.useEffect(() => {
    if (page.data?.reports)
      setKnown((previous) => {
        const entries = new Map(
          previous.map((r) => [
            summaryKey(r.owner, r.summary_id) + r.pubkey,
            r,
          ]),
        );
        for (const row of page.data.reports)
          entries.set(summaryKey(row.owner, row.summary_id) + row.pubkey, row);
        return [...entries.values()];
      });
  }, [page.data]);
  React.useEffect(
    () => () => {
      cache.removeQueries({
        queryKey: ["summary-library", scope.relayUrl, scope.owner],
      });
    },
    [cache, scope.relayUrl, scope.owner],
  );
  const settling =
    query.trim() !== debounced || page.isFetching || invalidDates;
  const selectedReports = reports.filter((r) => selected.includes(r.id));
  const change = useMutation({
    mutationFn: ({
      items,
      operation,
    }: {
      items: SummaryReportItem[];
      operation: string;
    }) =>
      summaryOperation(scope, {
        summary_operation: operation,
        event_ids: items.map((r) => r.id),
      }),
    onSuccess: async (_, variables) => {
      setSelected([]);
      setDeleting([]);
      setOpened(null);
      await Promise.all([
        cache.invalidateQueries({
          queryKey: ["summary-library", scope.relayUrl, scope.owner],
        }),
        cache.invalidateQueries({
          queryKey: ["summary-reports", scope.relayUrl, scope.owner],
        }),
        cache.invalidateQueries({
          queryKey: ["summary-overview", scope.relayUrl, scope.owner],
        }),
      ]);
      if (variables.operation !== "reports_delete")
        toast(
          variables.operation === "reports_archive"
            ? "Reports archived"
            : "Reports restored",
          {
            action: {
              label: "Undo",
              onClick: () =>
                change.mutate({
                  items: variables.items,
                  operation:
                    variables.operation === "reports_archive"
                      ? "reports_restore"
                      : "reports_archive",
                }),
            },
          },
        );
    },
  });
  const archive = (items: SummaryReportItem[]) =>
    change.mutate({
      items,
      operation: archived ? "reports_restore" : "reports_archive",
    });
  const options = new Map(
    definitions.map((r) => [
      summaryKey(scope.owner, r.config.summary_id),
      r.config.name,
    ]),
  );
  for (const r of known)
    options.set(
      summaryKey(r.owner, r.summary_id),
      `${r.summary_name}${r.owner === scope.owner ? "" : ` · ${r.owner_name}`}`,
    );
  const agentOptions = new Map(agents.map((a) => [a.pubkey, a.name]));
  for (const r of known) agentOptions.set(r.pubkey, r.agent_name);
  const ownerOptions = new Map([[scope.owner, "Me"]]);
  for (const r of known)
    if (r.owner !== scope.owner) ownerOptions.set(r.owner, r.owner_name);
  const rowLabel = (r: SummaryReportItem) =>
    `${r.title}, ${r.summary_name}, ${new Date(r.created_at * 1000).toLocaleString()}`;
  const close = () => {
    setOpened(null);
    setTimeout(() => {
      const element = document.getElementById(
        `${labelId}-report-${lastOpened.current}`,
      );
      element?.focus();
      element?.scrollIntoView({ block: "nearest" });
    }, 0);
  };
  const controls = (r: SummaryReportItem) => (
    <>
      <Button
        size="sm"
        variant="ghost"
        disabled={change.isPending || settling}
        aria-label={`${archived ? "Restore" : "Archive"} ${rowLabel(r)}`}
        onClick={() => archive([r])}
      >
        {archived ? "Restore" : "Archive"}
      </Button>
      {r.can_delete && (
        <Button
          size="sm"
          variant="ghost"
          disabled={change.isPending || settling}
          aria-label={`Delete ${rowLabel(r)}`}
          onClick={() => setDeleting([r])}
        >
          Delete
        </Button>
      )}
    </>
  );
  return (
    <section
      className="space-y-4"
      aria-label={archived ? "Archived reports" : "Reports"}
    >
      {change.isError && (
        <p role="alert">Could not update reports. {message(change.error)}</p>
      )}
      {opened && (
        <SummaryReportReader
          scope={scope}
          item={opened}
          onClose={close}
          actions={controls(opened)}
          onRegenerate={
            opened.owner === scope.owner &&
            definitions.some(
              (r) =>
                r.config.summary_id === opened.summary_id && !r.config.archived,
            ) &&
            opened.since != null &&
            opened.until != null
              ? async () => {
                  const record = definitions.find(
                    (r) => r.config.summary_id === opened.summary_id,
                  );
                  if (!record)
                    throw new Error(
                      "This Summary setup is no longer available.",
                    );
                  const result = await runNamedSummary(
                    scope.owner,
                    scope.relayUrl,
                    record,
                    opened.id,
                  );
                  if ("alreadyRunning" in result && result.alreadyRunning)
                    throw new Error(
                      "This Summary already has an active run. Finish or cancel it before regenerating a report.",
                    );
                  await cache.invalidateQueries({
                    queryKey: [
                      "summary-runs",
                      scope.relayUrl,
                      scope.owner,
                      opened.summary_id,
                    ],
                  });
                }
              : undefined
          }
        />
      )}
      <div hidden={Boolean(opened)} className="space-y-4">
        <div>
          <h3 className="font-semibold">
            {archived ? "Archived reports" : "Reports"}
          </h3>
          <p className="text-sm text-muted-foreground">
            {archived
              ? "Your archived reports. Restoring them keeps other recipients' organization unchanged."
              : "Browse report history. Archiving a report does not stop its Summary schedule."}
          </p>
        </div>
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-3">
          <label className="space-y-1 text-sm" htmlFor={`${labelId}-search`}>
            Search reports
            <Input
              id={`${labelId}-search`}
              aria-label="Search reports"
              placeholder="Report text, Summary, owner or agent…"
              maxLength={200}
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                reset();
              }}
            />
          </label>
          {!summaryId && (
            <label className="space-y-1 text-sm" htmlFor={`${labelId}-summary`}>
              Summary
              <SummaryFilterSelect
                id={`${labelId}-summary`}
                label="Filter by Summary"
                value={summary}
                onChange={(v) => {
                  setSummary(v);
                  reset();
                }}
                options={[
                  { value: "all", label: "All Summaries, including shared" },
                  ...[...options].map(([value, label]) => ({ value, label })),
                ]}
              />
            </label>
          )}
          <label className="space-y-1 text-sm" htmlFor={`${labelId}-agent`}>
            Agent
            <SummaryFilterSelect
              id={`${labelId}-agent`}
              label="Filter by agent"
              value={reporter}
              onChange={(v) => {
                setReporter(v);
                reset();
              }}
              options={[
                { value: "all", label: "All agents" },
                ...[...agentOptions].map(([value, label]) => ({
                  value,
                  label,
                })),
              ]}
            />
          </label>
          {!summaryId && (
            <label className="space-y-1 text-sm" htmlFor={`${labelId}-owner`}>
              Shared by
              <SummaryFilterSelect
                id={`${labelId}-owner`}
                label="Filter by owner"
                value={owner}
                onChange={(v) => {
                  setOwner(v);
                  setSummary("all");
                  reset();
                }}
                options={[
                  { value: "all", label: "Everyone" },
                  ...[...ownerOptions].map(([value, label]) => ({
                    value,
                    label,
                  })),
                ]}
              />
            </label>
          )}
          <label className="space-y-1 text-sm" htmlFor={`${labelId}-from`}>
            Published from
            <Input
              id={`${labelId}-from`}
              aria-label="Reports from"
              type="date"
              value={since}
              onChange={(e) => {
                setSince(e.target.value);
                reset();
              }}
            />
          </label>
          <label className="space-y-1 text-sm" htmlFor={`${labelId}-until`}>
            Published through
            <Input
              id={`${labelId}-until`}
              aria-label="Reports through"
              type="date"
              value={until}
              min={since || undefined}
              onChange={(e) => {
                setUntil(e.target.value);
                reset();
              }}
            />
          </label>
          <Button
            variant="ghost"
            className="self-end"
            onClick={() => {
              setQuery("");
              setSummary(
                summaryId
                  ? summaryKey(summaryOwner ?? scope.owner, summaryId)
                  : "all",
              );
              setReporter("all");
              setOwner("all");
              setSince("");
              setUntil("");
              reset();
            }}
          >
            Clear filters
          </Button>
        </div>
        {invalidDates && (
          <p role="alert">Choose an end date on or after the start date.</p>
        )}
        {page.isError && (
          <p role="alert">
            Could not load report history. {message(page.error)}{" "}
            <Button variant="outline" onClick={() => void page.refetch()}>
              Retry
            </Button>
          </p>
        )}
        {page.isLoading ? (
          <p>Loading reports…</p>
        ) : (
          <>
            <div className="flex flex-wrap items-center gap-3">
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  aria-label="Select this page"
                  disabled={settling || change.isPending}
                  checked={
                    reports.length > 0 &&
                    reports.every((r) => selected.includes(r.id))
                  }
                  onChange={(e) =>
                    setSelected(
                      e.target.checked ? reports.map((r) => r.id) : [],
                    )
                  }
                />
                Select this page
              </label>
              <span className="text-sm text-muted-foreground">
                {selectedReports.length} selected
              </span>
              <Button
                size="sm"
                variant="outline"
                disabled={
                  change.isPending || !selectedReports.length || page.isError
                }
                onClick={() => archive(selectedReports)}
              >
                {archived ? "Restore selected" : "Archive selected"}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={
                  change.isPending ||
                  settling ||
                  !selectedReports.length ||
                  !selectedReports.every((r) => r.can_delete)
                }
                onClick={() => setDeleting(selectedReports)}
              >
                Delete selected
              </Button>
            </div>
            <div className="divide-y rounded-xl border">
              {reports.map((r) => (
                <div key={r.id} className="flex items-center gap-3 p-3">
                  <input
                    type="checkbox"
                    aria-label={`Select ${rowLabel(r)}`}
                    checked={selected.includes(r.id)}
                    onChange={(e) =>
                      setSelected((ids) =>
                        e.target.checked
                          ? [...ids, r.id]
                          : ids.filter((id) => id !== r.id),
                      )
                    }
                  />
                  <button
                    id={`${labelId}-report-${r.id}`}
                    type="button"
                    className="min-w-0 flex-1 text-left"
                    onClick={() => {
                      lastOpened.current = r.id;
                      setOpened(r);
                    }}
                  >
                    <span className="block truncate text-sm font-medium">
                      {r.title}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {r.summary_name} ·{" "}
                      {r.owner === scope.owner ? "You" : r.owner_name}
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {new Date(r.created_at * 1000).toLocaleString()} ·{" "}
                      {r.agent_name}
                    </span>
                  </button>
                  {controls(r)}
                </div>
              ))}
              {!reports.length && !page.isError && !invalidDates && (
                <p className="p-5 text-sm text-muted-foreground">
                  No matching reports on this page.
                </p>
              )}
            </div>
            <div className="flex items-center justify-between">
              <Button
                variant="outline"
                disabled={cursors.length === 1}
                onClick={() => {
                  setCursors((v) => v.slice(0, -1));
                  setSelected([]);
                }}
              >
                Previous
              </Button>
              <span className="text-sm text-muted-foreground">
                Page {cursors.length}
              </span>
              <Button
                variant="outline"
                disabled={!page.data?.next || page.isError}
                onClick={() => {
                  setCursors((v) => [...v, page.data?.next ?? null]);
                  setSelected([]);
                }}
              >
                Next
              </Button>
            </div>
          </>
        )}
      </div>
      <AlertDialog
        open={deleting.length > 0}
        onOpenChange={(open) => {
          if (!open && !change.isPending) setDeleting([]);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Delete {deleting.length}{" "}
              {deleting.length === 1 ? "report" : "reports"}?
            </AlertDialogTitle>
            <AlertDialogDescription>
              This removes these reports from Buzz for every recipient. You
              cannot undo this action. Relay storage and downloaded copies may
              remain. Schedules and reading progress are preserved.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <ul className="max-h-48 space-y-2 overflow-auto text-sm">
            {deleting.map((r) => (
              <li key={r.id}>{rowLabel(r)}</li>
            ))}
          </ul>
          {change.isError && <p role="alert">{message(change.error)}</p>}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={change.isPending || settling}>
              Cancel
            </AlertDialogCancel>
            <Button
              disabled={change.isPending || settling}
              variant="destructive"
              onClick={() =>
                change.mutate({ items: deleting, operation: "reports_delete" })
              }
            >
              {change.isPending ? "Deleting…" : "Delete reports"}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
