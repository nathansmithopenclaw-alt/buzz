import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { validateDispatchSearch } from "@/features/dispatch/dispatchSearch";
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
  validateSearch: validateDispatchSearch,
});

function DispatchIssueRouteComponent() {
  usePreviewFeatureWarning("dispatch");
  const { issueId } = Route.useParams();
  const search = Route.useSearch();
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="dispatch" />}>
      <DispatchIssueDetailScreen
        issueId={issueId}
        returnSearch={search}
        workspaceId={search.workspace}
      />
    </React.Suspense>
  );
}
