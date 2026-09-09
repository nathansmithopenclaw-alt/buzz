import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import { useCommunities } from "@/features/communities/useCommunities";
import { parseSummaryLink } from "./summaryLink";
export function useOpenSummaryLink() {
  const navigate = useNavigate();
  const { activeCommunity } = useCommunities();
  return React.useCallback(
    (href: string) => {
      const summary = parseSummaryLink(href);
      if (!summary) return false;
      if (
        summary.relay.replace(/\/$/, "") !==
        activeCommunity?.relayUrl.replace(/\/$/, "")
      ) {
        toast.error(
          "Switch to the community where this Summary was shared, then open the link again.",
        );
        return true;
      }
      void navigate({ to: "/pulse", search: { summaryReport: summary.id } });
      return true;
    },
    [navigate, activeCommunity?.relayUrl],
  );
}
