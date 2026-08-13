import { Hash, Plus } from "lucide-react";

import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import type { DispatchWorkspace } from "../types";

const controlClassName =
  "flex items-center gap-1 rounded-full border border-border/45 bg-muted/55 px-2.5 py-1 text-xs text-muted-foreground";

export function DispatchWorkspaceControl({
  activeWorkspace,
  emptyWorkspaces,
  populatedWorkspaces,
  prefix,
  onSelect,
}: {
  activeWorkspace: DispatchWorkspace | null;
  emptyWorkspaces: DispatchWorkspace[];
  populatedWorkspaces: DispatchWorkspace[];
  prefix: string | null;
  onSelect: (workspaceId: string) => void;
}) {
  const activeIsEmpty = activeWorkspace?.trackerEventCount === 0;
  return (
    <div className="flex items-center gap-2">
      {activeIsEmpty || populatedWorkspaces.length === 1 ? (
        <span
          className={controlClassName}
          data-testid="dispatch-workspace-label"
        >
          <Hash className="h-3.5 w-3.5" />
          <span className="max-w-48 truncate">
            {activeIsEmpty ? "Starting · " : ""}
            {activeWorkspace?.name ?? populatedWorkspaces[0]?.name}
            {!activeIsEmpty && prefix ? ` · ${prefix}` : ""}
          </span>
        </span>
      ) : populatedWorkspaces.length > 1 ? (
        <label className={controlClassName}>
          <Hash className="h-3.5 w-3.5" />
          <select
            aria-label="View Dispatch workspace"
            className="max-w-48 bg-transparent text-xs text-muted-foreground outline-hidden"
            data-testid="dispatch-workspace-picker"
            onChange={(event) => onSelect(event.target.value)}
            value={activeWorkspace?.trackerEventCount ? activeWorkspace.id : ""}
          >
            <option value="">View workspace</option>
            {populatedWorkspaces.map((workspace) => (
              <option key={workspace.id} value={workspace.id}>
                {workspace.name}
                {workspace.id === activeWorkspace?.id && prefix
                  ? ` · ${prefix}`
                  : ""}
              </option>
            ))}
          </select>
        </label>
      ) : null}

      {activeIsEmpty && populatedWorkspaces.length === 1 ? (
        <Button
          data-testid="dispatch-view-populated-workspace"
          onClick={() => onSelect(populatedWorkspaces[0].id)}
          size="xs"
          variant="ghost"
        >
          View {populatedWorkspaces[0].name}
        </Button>
      ) : null}

      {activeIsEmpty && populatedWorkspaces.length > 1 ? (
        <label className={controlClassName}>
          <Hash className="h-3.5 w-3.5" />
          <select
            aria-label="View populated Dispatch workspace"
            className="max-w-48 bg-transparent text-xs text-muted-foreground outline-hidden"
            data-testid="dispatch-workspace-picker"
            onChange={(event) => onSelect(event.target.value)}
            value=""
          >
            <option value="">View workspace</option>
            {populatedWorkspaces.map((workspace) => (
              <option key={workspace.id} value={workspace.id}>
                {workspace.name}
              </option>
            ))}
          </select>
        </label>
      ) : null}

      {emptyWorkspaces.length > 0 ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              data-testid="dispatch-empty-workspace-init"
              size="xs"
              variant="ghost"
            >
              <Plus className="h-3.5 w-3.5" /> Start Dispatch
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-72">
            <DropdownMenuLabel>Start in an empty workspace</DropdownMenuLabel>
            <p className="px-2 pb-2 text-xs text-muted-foreground">
              New issues will live there. Existing issues stay in their current
              workspace.
            </p>
            <DropdownMenuSeparator />
            {emptyWorkspaces.map((workspace) => (
              <DropdownMenuItem
                data-testid={`dispatch-init-workspace-${workspace.id}`}
                key={workspace.id}
                onSelect={() => onSelect(workspace.id)}
              >
                <Hash /> {workspace.name}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  );
}
