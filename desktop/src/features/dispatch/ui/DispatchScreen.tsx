import * as React from "react";

import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { Button } from "@/shared/ui/button";

export type DispatchStubPanelProps = {
  workspaceId: string | null;
};

export function DispatchBoardPanelStub({
  workspaceId,
}: DispatchStubPanelProps) {
  return (
    <div
      data-testid="dispatch-board-mount"
      data-workspace-id={workspaceId ?? ""}
    />
  );
}

export function DispatchListPanelStub({ workspaceId }: DispatchStubPanelProps) {
  return (
    <div
      data-testid="dispatch-list-mount"
      data-workspace-id={workspaceId ?? ""}
    />
  );
}

export function DispatchTriagePanelStub({
  workspaceId,
}: DispatchStubPanelProps) {
  return (
    <div
      data-testid="dispatch-triage-mount"
      data-workspace-id={workspaceId ?? ""}
    />
  );
}

export function DispatchMyIssuesPanelStub({
  workspaceId,
}: DispatchStubPanelProps) {
  return (
    <div
      data-testid="dispatch-my-issues-mount"
      data-workspace-id={workspaceId ?? ""}
    />
  );
}

type DispatchView = "board" | "list" | "triage" | "my-issues";

export function DispatchScreen() {
  const [view, setView] = React.useState<DispatchView>("board");
  const workspaceId: string | null = null;
  return (
    <main
      className="flex h-full min-h-0 flex-col"
      data-testid="dispatch-screen"
    >
      <TopChromeInsetHeader data-tauri-drag-region>
        <div className="flex min-h-12 items-center justify-between gap-3 px-5">
          <h1 className="text-base font-semibold">Dispatch</h1>
          <nav aria-label="Dispatch views" className="flex items-center gap-1">
            {(["board", "list", "triage", "my-issues"] as const).map((item) => (
              <Button
                aria-pressed={view === item}
                key={item}
                onClick={() => setView(item)}
                size="sm"
                variant={view === item ? "secondary" : "ghost"}
              >
                {item === "my-issues"
                  ? "My Issues"
                  : item.charAt(0).toUpperCase() + item.slice(1)}
              </Button>
            ))}
          </nav>
        </div>
      </TopChromeInsetHeader>
      <section className="min-h-0 flex-1 overflow-auto p-5">
        {view === "board" ? (
          <DispatchBoardPanelStub workspaceId={workspaceId} />
        ) : null}
        {view === "list" ? (
          <DispatchListPanelStub workspaceId={workspaceId} />
        ) : null}
        {view === "triage" ? (
          <DispatchTriagePanelStub workspaceId={workspaceId} />
        ) : null}
        {view === "my-issues" ? (
          <DispatchMyIssuesPanelStub workspaceId={workspaceId} />
        ) : null}
      </section>
    </main>
  );
}
