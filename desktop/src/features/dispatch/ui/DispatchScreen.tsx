import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  AlertTriangle,
  LayoutGrid,
  List,
  Plus,
  RefreshCw,
  UserRound,
  Waypoints,
} from "lucide-react";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { cn } from "@/shared/lib/cn";
import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { Button } from "@/shared/ui/button";
import {
  DISPATCH_SEARCH_KEYS,
  dispatchFilterSearchPatch,
  dispatchFiltersFromSearch,
  dispatchSearchForNavigation,
  encodeExcludedIssueIds,
  parseExcludedIssueIds,
  type DispatchView,
} from "../dispatchSearch";
import { useDispatchContext, useDispatchWorkspaces } from "../hooks";
import type { DispatchWorkspace } from "../types";
import type {
  DispatchFilters,
  DispatchGroupBy,
  DispatchSortKey,
} from "../viewModel";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import {
  DispatchBoardPanel,
  DispatchListPanel,
  DispatchPanelsTooltipProvider,
} from "./DispatchPanels";
import { MyIssues } from "./MyIssues";
import { NewIssueDialog } from "./NewIssueDialog";
import { TriageInbox } from "./TriageInbox";
import { DispatchWorkspaceControl } from "./DispatchWorkspaceControl";

const LAST_WORKSPACE_KEY = "buzz-dispatch-last-workspace-v1";

function initialWorkspaceId(workspaces: DispatchWorkspace[]): string | null {
  const persisted = getStorageItem(LAST_WORKSPACE_KEY);
  if (persisted && workspaces.some((workspace) => workspace.id === persisted)) {
    return persisted;
  }
  return workspaces.length === 1 ? workspaces[0].id : null;
}

function filtersForGroup(
  filters: DispatchFilters,
  groupBy: DispatchGroupBy,
  groupId: string,
): DispatchFilters {
  if (groupBy === "status") {
    return { ...filters, status: groupId as DispatchFilters["status"] };
  }
  if (groupBy === "priority") {
    return { ...filters, priority: groupId as DispatchFilters["priority"] };
  }
  if (groupBy === "assignee") return { ...filters, assignee: groupId };
  return { ...filters, project: groupId };
}

function workspacePrefix(identifiers: string[]): string | null {
  for (const identifier of identifiers) {
    const match = /^([A-Za-z][A-Za-z0-9]{0,11})-/.exec(identifier);
    if (match) return match[1].toUpperCase();
  }
  return null;
}

function fallbackWorkspacePrefix(name: string): string {
  const prefix = name
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .map((word) => word[0]?.toUpperCase())
    .join("")
    .slice(0, 4);
  return prefix || "TRK";
}

function syncedLabel(snapshotAt: string): string {
  const date = new Date(snapshotAt);
  if (Number.isNaN(date.valueOf()) || date.valueOf() === 0) return "Not synced";
  return `Synced ${new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
  }).format(date)}`;
}

const tabClassName =
  "relative flex h-9 items-center gap-1.5 px-3 text-sm text-muted-foreground transition-colors hover:text-foreground after:absolute after:inset-x-0 after:bottom-0 after:h-0.5 after:bg-transparent";

export function DispatchScreen() {
  const navigate = useNavigate();
  const { applyPatch, values } = useHistorySearchState(DISPATCH_SEARCH_KEYS);
  const identity = useIdentityQuery();
  const workspaceQuery = useDispatchWorkspaces();
  const { channels } = useChannelNavigation();
  const populatedWorkspaces = React.useMemo(
    () =>
      workspaceQuery.workspaces.filter(
        (workspace) => workspace.trackerEventCount > 0,
      ),
    [workspaceQuery.workspaces],
  );
  const emptyWorkspaces = React.useMemo(
    () =>
      workspaceQuery.workspaces.filter(
        (workspace) => workspace.trackerEventCount === 0,
      ),
    [workspaceQuery.workspaces],
  );
  // The workspace to fold is already known at t=0 — it is in the URL, or it is
  // the one persisted from last time. Waiting for discovery to confirm the id
  // exists puts the whole workspace probe on the critical path of the fold,
  // which is the bulk of the cold-open wait. Start the fold from the
  // unconfirmed id instead and let discovery reconcile.
  //
  // Only trusted while discovery is in flight: once the list lands it is
  // authoritative, so a stale id falls back to null and the effect below
  // re-picks. The screen renders its loading state over this window
  // (`workspaceQuery.isLoading` is checked before `context.error`), so a fold
  // that fails against a stale id is never shown.
  const unconfirmedWorkspaceId = React.useMemo(
    () => values.workspace ?? getStorageItem(LAST_WORKSPACE_KEY),
    [values.workspace],
  );
  const workspaceId =
    workspaceQuery.workspaces.find(
      (workspace) => workspace.id === values.workspace,
    )?.id ?? (workspaceQuery.isLoading ? unconfirmedWorkspaceId : null);
  const activeWorkspace =
    workspaceQuery.workspaces.find(
      (workspace) => workspace.id === workspaceId,
    ) ?? null;
  const view = (values.view as DispatchView | null) ?? "board";
  const filters = dispatchFiltersFromSearch(values);
  const groupBy = (values.group as DispatchGroupBy | null) ?? "status";
  const sort = {
    key: (values.sort as DispatchSortKey | null) ?? "identifier",
    direction: (values.direction as "asc" | "desc" | null) ?? "asc",
  };
  const excludedIssueIds = parseExcludedIssueIds(values.exclude);
  const [newIssueOpen, setNewIssueOpen] = React.useState(false);

  React.useEffect(() => {
    if (workspaceQuery.isLoading || workspaceId) return;
    const initial = initialWorkspaceId(populatedWorkspaces);
    if (initial) applyPatch({ workspace: initial }, { replace: true });
  }, [applyPatch, populatedWorkspaces, workspaceId, workspaceQuery.isLoading]);

  const selectWorkspace = (next: string) => {
    applyPatch({ workspace: next, exclude: null }, { replace: true });
    setStorageItem(LAST_WORKSPACE_KEY, next);
  };
  const context = useDispatchContext(workspaceId);
  const assigneePubkeys = React.useMemo(
    () =>
      [
        ...new Set(
          context.issues.map((issue) => issue.assignee).filter(Boolean),
        ),
      ] as string[],
    [context.issues],
  );
  const profilesQuery = useUsersBatchQuery(assigneePubkeys);
  const profiles = profilesQuery.data?.profiles ?? {};
  const prefix = workspacePrefix(
    context.issues.map((issue) => issue.identifier),
  );
  const selectedWorkspace = workspaceQuery.workspaces.find(
    (workspace) => workspace.id === workspaceId,
  );
  const createPrefix =
    prefix ?? fallbackWorkspacePrefix(selectedWorkspace?.name ?? "");
  const triageCount = context.issues.filter(
    (issue) => issue.status === "triage",
  ).length;
  const openIssue = (issueId: string) => {
    if (!workspaceId) return;
    void navigate({
      to: "/dispatch/issue/$issueId",
      params: { issueId },
      search: dispatchSearchForNavigation(values, workspaceId),
    });
  };
  const changeView = (next: DispatchView) =>
    applyPatch(
      { view: next === "board" ? null : next, exclude: null },
      { replace: true },
    );
  const changeFilters = (next: DispatchFilters) =>
    applyPatch(
      { ...dispatchFilterSearchPatch(next), exclude: null },
      { replace: true },
    );

  return (
    <DispatchPanelsTooltipProvider>
      <main
        className="flex h-full min-h-0 flex-col"
        data-testid="dispatch-screen"
      >
        <TopChromeInsetHeader data-tauri-drag-region>
          <div className="flex min-h-12 items-center gap-3 px-5">
            <h1 className="flex items-center gap-2 text-base font-semibold">
              <Waypoints className="h-4 w-4" /> Dispatch
            </h1>
            <DispatchWorkspaceControl
              activeWorkspace={activeWorkspace}
              emptyWorkspaces={emptyWorkspaces}
              onSelect={selectWorkspace}
              populatedWorkspaces={populatedWorkspaces}
              prefix={prefix}
            />
            <div className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
              {view !== "my-issues" ? (
                <Button
                  data-testid="dispatch-new-issue"
                  disabled={!workspaceId || context.viewerRole === "none"}
                  onClick={() => setNewIssueOpen(true)}
                  size="sm"
                >
                  <Plus /> New Issue
                </Button>
              ) : null}
              <span
                className={cn(context.isRetrying && "text-warning")}
                data-testid={
                  context.isRetrying
                    ? "dispatch-rate-limit-retrying"
                    : undefined
                }
              >
                {context.isRetrying
                  ? "Relay busy · retrying…"
                  : syncedLabel(context.snapshotAt)}
              </span>
              <Button
                aria-label="Refresh Dispatch"
                data-testid="dispatch-refresh"
                disabled={workspaceQuery.isLoading || context.isLoading}
                onClick={() => {
                  // Refresh what the screen shows, which includes the
                  // workspace list: it is cached with a staleTime and nothing
                  // invalidates it, so this is the only way to pick up a
                  // newly-joined workspace before that expires.
                  workspaceQuery.refresh();
                  if (workspaceId) void context.refresh();
                }}
                size="icon-xs"
                variant="ghost"
              >
                <RefreshCw
                  className={cn(
                    (workspaceQuery.isLoading || context.isLoading) &&
                      "animate-spin",
                  )}
                />
              </Button>
            </div>
          </div>
          <nav
            aria-label="Dispatch views"
            className="flex border-b border-border/45 px-5"
          >
            <button
              aria-pressed={view === "board"}
              className={cn(
                tabClassName,
                view === "board" && "text-foreground after:bg-primary",
              )}
              data-testid="dispatch-tab-board"
              onClick={() => changeView("board")}
              type="button"
            >
              <LayoutGrid className="h-4 w-4" /> Board
            </button>
            <button
              aria-pressed={view === "list"}
              className={cn(
                tabClassName,
                view === "list" && "text-foreground after:bg-primary",
              )}
              data-testid="dispatch-tab-list"
              onClick={() => changeView("list")}
              type="button"
            >
              <List className="h-4 w-4" /> List
            </button>
            <button
              aria-pressed={view === "triage"}
              className={cn(
                tabClassName,
                view === "triage" && "text-foreground after:bg-primary",
              )}
              onClick={() => changeView("triage")}
              type="button"
            >
              <AlertTriangle className="h-4 w-4" /> Triage
              {triageCount > 0 ? (
                <span className="rounded-full bg-muted px-1.5 py-0.5 text-2xs text-muted-foreground">
                  {triageCount}
                </span>
              ) : null}
            </button>
            <button
              aria-pressed={view === "my-issues"}
              className={cn(
                tabClassName,
                view === "my-issues" && "text-foreground after:bg-primary",
              )}
              onClick={() => changeView("my-issues")}
              type="button"
            >
              <UserRound className="h-4 w-4" /> My Issues
            </button>
          </nav>
        </TopChromeInsetHeader>

        {context.diagnostics.length > 0 ? (
          <div
            className="flex items-center gap-2 border-b border-warning/30 bg-warning-bg px-5 py-2 text-xs text-warning"
            data-testid="dispatch-diagnostics-warning"
          >
            <AlertTriangle className="h-4 w-4" />
            {context.diagnostics.length} fold warning
            {context.diagnostics.length === 1 ? "" : "s"}; conflicting history
            was preserved for review.
          </div>
        ) : null}

        {workspaceQuery.isLoading ? (
          <div className="m-auto text-sm text-muted-foreground">
            Discovering Tracker workspaces…
          </div>
        ) : workspaceQuery.error ? (
          <div className="m-auto max-w-md text-center text-sm text-destructive">
            {workspaceQuery.error.message}
          </div>
        ) : workspaceQuery.workspaces.length === 0 ? (
          <div className="m-auto max-w-md text-center text-sm text-muted-foreground">
            No Tracker-enabled member workspaces were found for this identity.
          </div>
        ) : populatedWorkspaces.length === 0 && !workspaceId ? (
          <div className="m-auto max-w-md text-center text-sm text-muted-foreground">
            No populated Dispatch workspaces yet. Use Start Dispatch above to
            choose where new issues will live.
          </div>
        ) : !workspaceId ? (
          <div className="m-auto max-w-md text-center text-sm text-muted-foreground">
            Choose a workspace above to view its issues. This changes only the
            current view; it never moves issues between workspaces.
          </div>
        ) : context.isRetrying && context.isLoading ? (
          <div
            className="m-auto text-sm text-warning"
            data-testid="dispatch-rate-limit-retrying-loading"
          >
            Relay is busy; retrying Tracker history…
          </div>
        ) : context.error ? (
          <div className="m-auto max-w-md text-center text-sm text-destructive">
            {context.error.message}
          </div>
        ) : context.isLoading ? (
          <div className="m-auto text-sm text-muted-foreground">
            Folding Tracker history…
          </div>
        ) : (
          <section className="flex min-h-0 flex-1 flex-col overflow-auto">
            {view === "board" ? (
              <DispatchBoardPanel
                filters={filters}
                groupBy={groupBy}
                issues={context.issues}
                onFiltersChange={changeFilters}
                onGroupByChange={(next) =>
                  applyPatch(
                    { group: next === "status" ? null : next, exclude: null },
                    { replace: true },
                  )
                }
                onOpenIssue={openIssue}
                onShowMore={({
                  excludedIssueIds: excluded,
                  groupBy,
                  groupId,
                }) => {
                  const groupedFilters = filtersForGroup(
                    filters,
                    groupBy,
                    groupId,
                  );
                  applyPatch(
                    {
                      ...dispatchFilterSearchPatch(groupedFilters),
                      view: "list",
                      sort: "priority",
                      direction: null,
                      exclude: encodeExcludedIssueIds(excluded),
                    },
                    { replace: true },
                  );
                }}
                profiles={profiles}
                projects={context.projects}
              />
            ) : null}
            {view === "list" ? (
              <DispatchListPanel
                excludedIssueIds={excludedIssueIds}
                filters={filters}
                issues={context.issues}
                onClearOverflow={() =>
                  applyPatch({ exclude: null }, { replace: true })
                }
                onFiltersChange={changeFilters}
                onOpenIssue={openIssue}
                onSortChange={(next) =>
                  applyPatch(
                    {
                      sort: next.key === "identifier" ? null : next.key,
                      direction: next.direction === "asc" ? null : "desc",
                    },
                    { replace: true },
                  )
                }
                profiles={profiles}
                projects={context.projects}
                sort={sort}
              />
            ) : null}
            {view === "triage" ? (
              <TriageInbox
                channels={channels}
                error={context.error}
                isLoading={context.isLoading || workspaceQuery.isLoading}
                issues={context.issues}
                onOpenIssue={openIssue}
                onRetry={() => void context.refresh()}
                profiles={profiles}
                projects={context.projects}
              />
            ) : null}
            {view === "my-issues" ? (
              <MyIssues
                channels={channels}
                currentPubkey={identity.data?.pubkey}
                isLoading={context.isLoading || workspaceQuery.isLoading}
                issues={context.issues}
                onOpenIssue={openIssue}
                profiles={profiles}
                projects={context.projects}
              />
            ) : null}
          </section>
        )}
        {workspaceId ? (
          <NewIssueDialog
            identifierPrefix={createPrefix}
            onOpenChange={setNewIssueOpen}
            open={newIssueOpen}
            projects={context.projects}
            refresh={context.refresh}
            streamId={workspaceId}
          />
        ) : null}
      </main>
    </DispatchPanelsTooltipProvider>
  );
}
