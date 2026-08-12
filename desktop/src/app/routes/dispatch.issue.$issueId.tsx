import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const DispatchIssueDetailStub = React.lazy(async () => {
  const module = await import("@/features/dispatch/ui/DispatchIssueDetailStub");
  return { default: module.DispatchIssueDetailStub };
});

export const Route = createFileRoute("/dispatch/issue/$issueId")({
  component: DispatchIssueRouteComponent,
});

function DispatchIssueRouteComponent() {
  usePreviewFeatureWarning("dispatch");
  const { issueId } = Route.useParams();
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="dispatch" />}>
      <DispatchIssueDetailStub issueId={issueId} />
    </React.Suspense>
  );
}
