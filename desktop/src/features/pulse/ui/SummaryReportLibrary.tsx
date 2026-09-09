import * as React from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/shared/ui/button";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogCancel,
  AlertDialogAction,
} from "@/shared/ui/alert-dialog";
import { Input } from "@/shared/ui/input";
import { SummaryFilterSelect } from "./SummaryFilterSelect";
import type { RelayEvent } from "@/shared/api/types";
import {
  summaryOperation,
  type SummaryScope,
  type SummaryDefinitionRecord,
} from "../lib/namedSummaries";
import { SummaryReportBrowser, reportContent } from "./SummaryReportBrowser";

type Cursor = { before_time: number; before_id: string };
type Page = { events: RelayEvent[]; next: Cursor | null };

/** Server-filtered, bounded pages; never retain private results across a failed refresh. */
export function SummaryReportLibrary({
  scope,
  definitions,
  agents,
  archived = false,
  summaryId,
}: {
  scope: SummaryScope;
  definitions: SummaryDefinitionRecord[];
  agents: { pubkey: string; name: string }[];
  archived?: boolean;
  summaryId?: string;
}) {
  const cache = useQueryClient();
  const labelId = React.useId();
  const [deleting, setDeleting] = React.useState<string[]>([]);
  const [query, setQuery] = React.useState("");
  const [summary, setSummary] = React.useState(summaryId ?? "all");
  const [reporter, setReporter] = React.useState("all");
  const [since, setSince] = React.useState("");
  const [until, setUntil] = React.useState("");
  const [cursors, setCursors] = React.useState<(Cursor | null)[]>([null]);
  const [selected, setSelected] = React.useState<string[]>([]);
  const [opened, setOpened] = React.useState<string | null>(null);
  const filter = {
    archived,
    query: query.trim() || undefined,
    owner: summary !== "all" ? scope.owner : undefined,
    summary_id: summary === "all" ? undefined : summary,
    reporter: reporter === "all" ? undefined : reporter,
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
    queryFn: () =>
      summaryOperation<Page>(scope, {
        summary_operation: "reports_list",
        filter,
      }),
    refetchInterval: 15000,
    staleTime: 0,
  });
  const events = page.isError ? [] : (page.data?.events ?? []);
  const reset = () => {
    setCursors([null]);
    setSelected([]);
    setOpened(null);
  };
  React.useEffect(
    () => () => {
      cache.removeQueries({
        queryKey: ["summary-library", scope.relayUrl, scope.owner],
      });
    },
    [cache, scope.relayUrl, scope.owner],
  );
  const change = useMutation({
    mutationFn: ({
      ids,
      remove = false,
    }: {
      ids: string[];
      remove?: boolean;
    }) =>
      summaryOperation(scope, {
        summary_operation: remove
          ? "reports_delete"
          : archived
            ? "reports_restore"
            : "reports_archive",
        event_ids: ids,
      }),
    onSuccess: async () => {
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
      ]);
    },
  });
  const canDelete = (e: RelayEvent) =>
    e.tags.some((t) => t[0] === "owner" && t[1] === scope.owner) ||
    (!e.tags.some((t) => t[0] === "owner") &&
      e.tags.some((t) => t[0] === "p" && t[1] === scope.owner));
  const selectedEvents = events.filter((e) => selected.includes(e.id));
  const chosen = events.find((e) => e.id === opened);
  return (
    <section
      className="space-y-4"
      aria-label={archived ? "Archived reports" : "Reports"}
    >
      <div>
        <h3 className="font-semibold">
          {archived ? "Archived reports" : "Reports"}
        </h3>
        <p className="text-sm text-muted-foreground">
          {archived
            ? "Your archived reports. Restore them at any time; other recipients keep their own organization."
            : "Browse report history. Archiving a report does not stop its Summary schedule."}
        </p>
      </div>
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-3">
        <label htmlFor={`${labelId}-search`} className="space-y-1 text-sm">
          Search reports
          <Input
            id={`${labelId}-search`}
            aria-label="Search reports"
            placeholder="Title or report text…"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              reset();
            }}
          />
        </label>
        {!summaryId && (
          <label htmlFor={`${labelId}-summary`} className="space-y-1 text-sm">
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
                ...definitions.map((r) => ({
                  value: r.config.summary_id,
                  label: r.config.name,
                })),
              ]}
            />
          </label>
        )}
        <label htmlFor={`${labelId}-agent`} className="space-y-1 text-sm">
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
              ...agents.map((a) => ({ value: a.pubkey, label: a.name })),
            ]}
          />
        </label>
        <label htmlFor={`${labelId}-from`} className="space-y-1 text-sm">
          From
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
        <label htmlFor={`${labelId}-through`} className="space-y-1 text-sm">
          Through
          <Input
            id={`${labelId}-through`}
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
            setSummary(summaryId ?? "all");
            setReporter("all");
            setSince("");
            setUntil("");
            reset();
          }}
        >
          Clear filters
        </Button>
      </div>
      {page.isError && (
        <p role="alert">
          Could not load report history. The relay must support report archives.{" "}
          <Button variant="outline" onClick={() => void page.refetch()}>
            Retry
          </Button>
        </p>
      )}
      {change.isError && (
        <p role="alert">
          Could not update the selected reports. Refresh and try again.
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
                checked={
                  events.length > 0 &&
                  events.every((e) => selected.includes(e.id))
                }
                onChange={(e) =>
                  setSelected(e.target.checked ? events.map((e) => e.id) : [])
                }
              />
              Select this page
            </label>
            <span className="text-sm text-muted-foreground">
              {selected.filter((id) => events.some((e) => e.id === id)).length}{" "}
              selected
            </span>
            <Button
              size="sm"
              variant="outline"
              disabled={change.isPending || !selected.length || page.isError}
              onClick={() =>
                change.mutate({
                  ids: selected.filter((id) => events.some((e) => e.id === id)),
                })
              }
            >
              {archived ? "Restore selected" : "Archive selected"}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={
                change.isPending ||
                !selectedEvents.length ||
                !selectedEvents.every(canDelete)
              }
              onClick={() => setDeleting(selectedEvents.map((e) => e.id))}
            >
              Delete selected
            </Button>
          </div>
          <div className="divide-y rounded-xl border">
            {events.map((event) => (
              <div key={event.id} className="flex items-center gap-3 p-3">
                <input
                  type="checkbox"
                  aria-label={`Select ${reportContent(event)?.title ?? "report"}`}
                  checked={selected.includes(event.id)}
                  onChange={(e) =>
                    setSelected((ids) =>
                      e.target.checked
                        ? [...ids, event.id]
                        : ids.filter((id) => id !== event.id),
                    )
                  }
                />
                <button
                  type="button"
                  className="min-w-0 flex-1 text-left"
                  onClick={() => setOpened(event.id)}
                >
                  <span className="block truncate text-sm font-medium">
                    {reportContent(event)?.title ?? "Untitled report"}
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {new Date(event.created_at * 1000).toLocaleString()} ·{" "}
                    {agents.find((a) => a.pubkey === event.pubkey)?.name ??
                      "Shared agent"}
                  </span>
                </button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={change.isPending}
                  onClick={() => change.mutate({ ids: [event.id] })}
                >
                  {archived ? "Restore" : "Archive"}
                </Button>
                {canDelete(event) && (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={change.isPending}
                    onClick={() => setDeleting([event.id])}
                  >
                    Delete
                  </Button>
                )}
              </div>
            ))}
            {!events.length && !page.isError && (
              <p className="p-5 text-sm text-muted-foreground">
                No matching reports on this page.
                {page.data?.next
                  ? " Continue to the next page to check older accessible reports."
                  : ""}
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
                setOpened(null);
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
                setOpened(null);
              }}
            >
              Next
            </Button>
          </div>
          {chosen && (
            <div className="space-y-3">
              <Button variant="ghost" onClick={() => setOpened(null)}>
                Close report
              </Button>
              <SummaryReportBrowser events={[chosen]} />
            </div>
          )}
        </>
      )}
      <AlertDialog
        open={deleting.length > 0}
        onOpenChange={(open) => {
          if (!open) setDeleting([]);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Delete {deleting.length}{" "}
              {deleting.length === 1 ? "report" : "reports"}?
            </AlertDialogTitle>
            <AlertDialogDescription>
              This removes the selected reports from Buzz for every recipient.
              You cannot undo this action. Existing downloaded copies and relay
              backups may remain. Your Summary schedules and reading progress
              are preserved.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => change.mutate({ ids: deleting, remove: true })}
            >
              Delete reports
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
