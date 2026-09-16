import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import { useCommunities } from "@/features/communities/useCommunities";
import { parseDispatchLink } from "../dispatchLink";
export function useOpenDispatchLink() {
  const navigate = useNavigate(),
    { activeCommunity } = useCommunities();
  return React.useCallback(
    (href: string) => {
      const link = parseDispatchLink(href);
      if (!link) return false;
      if (
        link.relay.replace(/\/$/, "") !==
        activeCommunity?.relayUrl.replace(/\/$/, "")
      ) {
        toast.error(
          "Switch to the community where this issue was shared, then open the link again.",
        );
        return true;
      }
      void navigate({
        to: "/dispatch/issue/$issueId",
        params: { issueId: link.issue },
        search: { workspace: link.workspace },
      });
      return true;
    },
    [navigate, activeCommunity?.relayUrl],
  );
}
export function DispatchLinkAnchor({
  href,
  children,
}: {
  href: string;
  children: React.ReactNode;
}) {
  const open = useOpenDispatchLink();
  return (
    <a
      href={href}
      className="underline"
      onClick={(event) => {
        event.preventDefault();
        open(href);
      }}
    >
      {children}
    </a>
  );
}
