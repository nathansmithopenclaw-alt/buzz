import type { ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { getStorageItem } from "@/shared/lib/safeStorage";
import { useDispatchContext, useDispatchWorkspaces } from "../hooks";
import { activeRecords } from "../recordTypes";
import { DispatchAutomationPanel } from "./DispatchAutomationPanel";

/** Fork-only operator controls for the currently selected Dispatch workspace. */
export function DispatchServiceSettings({
  children,
}: {
  children: (panel: ReactNode) => ReactNode;
}) {
  const navigate = useNavigate();
  const identity = useIdentityQuery();
  const viewer = identity.data?.pubkey;
  const { activeCommunity } = useCommunities();
  const { workspaces } = useDispatchWorkspaces();
  const preferred = getStorageItem("buzz-dispatch-last-workspace-v1");
  const populated = workspaces.filter(
    (workspace) => workspace.trackerEventCount > 0,
  );
  const workspace =
    workspaces.find((item) => item.id === preferred) ??
    (populated.length === 1 ? populated[0] : undefined);
  const context = useDispatchContext(workspace?.id ?? null);
  // Tracker maps workspace owner/admin to maintainer. Use the confirmed fold,
  // never a cached preview, as the authority for exposing operator controls.
  const available =
    workspace &&
    viewer &&
    !context.isLoading &&
    !context.isPreview &&
    !context.error &&
    context.viewerRole === "maintainer" &&
    activeRecords(context.records, "automation").length > 0;
  return children(
    available ? (
      <div key={`${activeCommunity?.relayUrl}:${workspace.id}:${viewer}`}>
        <h1 className="text-xl font-semibold">Dispatch service</h1>
        <p className="mt-2 text-sm text-muted-foreground">
          External service controls for {workspace.name}.
        </p>
        <DispatchAutomationPanel
          context={context}
          workspace={workspace.id}
          viewer={viewer}
          refresh={context.refresh}
          onOpenIssue={(issueId) =>
            void navigate({
              to: "/dispatch/issue/$issueId",
              params: { issueId },
              search: { workspace: workspace.id },
            })
          }
        />
      </div>
    ) : null,
  );
}
