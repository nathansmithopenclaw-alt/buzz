import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const DispatchIssueDetailScreen = React.lazy(async () => {
  const module = await import(
    "@/features/dispatch/ui/DispatchIssueDetailScreen"
  );
  return { default: module.DispatchIssueDetailScreen };
});

export const Route = createFileRoute("/dispatch/issue/$issueId")({
  component: DispatchIssueRouteComponent,
  validateSearch: (search: Record<string, unknown>) => ({
    workspace:
      typeof search.workspace === "string" ? search.workspace : undefined,
  }),
});

function DispatchIssueRouteComponent() {
  usePreviewFeatureWarning("dispatch");
  const { issueId } = Route.useParams();
  const { workspace } = Route.useSearch();
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="dispatch" />}>
      <DispatchIssueDetailScreen issueId={issueId} workspaceId={workspace} />
    </React.Suspense>
  );
}
