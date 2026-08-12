import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const DispatchScreen = React.lazy(async () => {
  const module = await import("@/features/dispatch/ui/DispatchScreen");
  return { default: module.DispatchScreen };
});

export const Route = createFileRoute("/dispatch")({
  component: DispatchRouteComponent,
});

function DispatchRouteComponent() {
  usePreviewFeatureWarning("dispatch");
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="dispatch" />}>
      <DispatchScreen />
    </React.Suspense>
  );
}
