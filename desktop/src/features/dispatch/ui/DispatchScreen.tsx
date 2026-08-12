import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  AlertTriangle,
  Hash,
  LayoutGrid,
  List,
  RefreshCw,
  UserRound,
  Waypoints,
} from "lucide-react";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { useDispatchContext, useDispatchWorkspaces } from "../hooks";
import type { DispatchWorkspace } from "../types";
import type { DispatchProfileMap } from "../viewModel";
import {
  DispatchBoardPanel,
  DispatchListPanel,
  DispatchPanelsTooltipProvider,
} from "./DispatchPanels";

const LAST_WORKSPACE_KEY = "buzz-dispatch-last-workspace-v1";

export type DispatchStubPanelProps = {
  workspaceId: string | null;
};

// Stable P1-B mount contracts. Patch replaces only these exports.
export function DispatchTriagePanelStub({
  workspaceId,
}: DispatchStubPanelProps) {
  return (
    <div
      className="p-5 text-sm text-muted-foreground"
      data-testid="dispatch-triage-mount"
      data-workspace-id={workspaceId ?? ""}
    >
      Triage is loading in the detail lane.
    </div>
  );
}

export function DispatchMyIssuesPanelStub({
  workspaceId,
}: DispatchStubPanelProps) {
  return (
    <div
      className="p-5 text-sm text-muted-foreground"
      data-testid="dispatch-my-issues-mount"
      data-workspace-id={workspaceId ?? ""}
    >
      My Issues is loading in the detail lane.
    </div>
  );
}

type DispatchView = "board" | "list" | "triage" | "my-issues";

function initialWorkspaceId(workspaces: DispatchWorkspace[]): string | null {
  const persisted = getStorageItem(LAST_WORKSPACE_KEY);
  if (persisted && workspaces.some((workspace) => workspace.id === persisted))
    return persisted;
  const withData = workspaces.filter(
    (workspace) => workspace.trackerEventCount > 0,
  );
  return withData.length === 1 ? withData[0].id : null;
}

function workspacePrefix(identifiers: string[]): string | null {
  for (const identifier of identifiers) {
    const match = /^([A-Za-z][A-Za-z0-9]{0,11})-/.exec(identifier);
    if (match) return match[1].toUpperCase();
  }
  return null;
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
  const workspaceQuery = useDispatchWorkspaces();
  const [view, setView] = React.useState<DispatchView>("board");
  const [workspaceId, setWorkspaceId] = React.useState<string | null>(null);
  const initializedFor = React.useRef<string | null>(null);

  React.useEffect(() => {
    const key = workspaceQuery.workspaces
      .map((workspace) => workspace.id)
      .join(",");
    if (workspaceQuery.isLoading || initializedFor.current === key) return;
    initializedFor.current = key;
    setWorkspaceId((current) => {
      if (
        current &&
        workspaceQuery.workspaces.some((workspace) => workspace.id === current)
      )
        return current;
      return initialWorkspaceId(workspaceQuery.workspaces);
    });
  }, [workspaceQuery.isLoading, workspaceQuery.workspaces]);

  const selectWorkspace = (next: string | null) => {
    setWorkspaceId(next);
    if (next) setStorageItem(LAST_WORKSPACE_KEY, next);
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
  const profiles = (profilesQuery.data?.profiles ?? {}) as DispatchProfileMap;
  const prefix = workspacePrefix(
    context.issues.map((issue) => issue.identifier),
  );
  const triageCount = context.issues.filter(
    (issue) => issue.status === "triage",
  ).length;
  const openIssue = (issueId: string) =>
    void navigate({ to: "/dispatch/issue/$issueId", params: { issueId } });

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
            {workspaceQuery.workspaces.length > 0 ? (
              <label className="flex items-center gap-1 rounded-full border border-border/45 bg-muted/55 px-2.5 py-1 text-xs text-muted-foreground">
                <Hash className="h-3.5 w-3.5" />
                <select
                  aria-label="Dispatch workspace"
                  className="max-w-48 bg-transparent text-xs text-muted-foreground outline-hidden"
                  data-testid="dispatch-workspace-picker"
                  onChange={(event) =>
                    selectWorkspace(event.target.value || null)
                  }
                  value={workspaceId ?? ""}
                >
                  <option value="">Select workspace</option>
                  {workspaceQuery.workspaces.map((candidate) => (
                    <option key={candidate.id} value={candidate.id}>
                      {candidate.name}
                      {candidate.id === workspaceId && prefix
                        ? ` · ${prefix}`
                        : ""}
                    </option>
                  ))}
                </select>
              </label>
            ) : null}
            <div className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
              <span>{syncedLabel(context.snapshotAt)}</span>
              <Button
                aria-label="Refresh Dispatch"
                data-testid="dispatch-refresh"
                disabled={!workspaceId || context.isLoading}
                onClick={context.refresh}
                size="icon-xs"
                variant="ghost"
              >
                <RefreshCw
                  className={cn(context.isLoading && "animate-spin")}
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
              onClick={() => setView("board")}
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
              onClick={() => setView("list")}
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
              onClick={() => setView("triage")}
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
              onClick={() => setView("my-issues")}
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
        ) : !workspaceId ? (
          <div className="m-auto max-w-md text-center text-sm text-muted-foreground">
            Choose a workspace above. Dispatch never assumes a production
            workspace.
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
          <section className="flex min-h-0 flex-1 flex-col">
            {view === "board" ? (
              <DispatchBoardPanel
                issues={context.issues}
                onOpenIssue={openIssue}
                profiles={profiles}
                projects={context.projects}
              />
            ) : null}
            {view === "list" ? (
              <DispatchListPanel
                issues={context.issues}
                onOpenIssue={openIssue}
                profiles={profiles}
                projects={context.projects}
              />
            ) : null}
            {view === "triage" ? (
              <DispatchTriagePanelStub workspaceId={workspaceId} />
            ) : null}
            {view === "my-issues" ? (
              <DispatchMyIssuesPanelStub workspaceId={workspaceId} />
            ) : null}
          </section>
        )}
      </main>
    </DispatchPanelsTooltipProvider>
  );
}
