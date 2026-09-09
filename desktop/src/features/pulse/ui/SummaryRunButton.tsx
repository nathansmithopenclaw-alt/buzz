import * as React from "react";
import { useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useCommunities } from "@/features/communities/useCommunities";
import { runSummary } from "../lib/runSummary";
import { Button } from "@/shared/ui/button";

export function SummaryRunButton({
  ownerPubkey,
  enabled,
  onRequested,
}: {
  ownerPubkey?: string;
  enabled: boolean;
  onRequested: () => void;
}) {
  const { activeCommunity } = useCommunities();
  const inFlight = React.useRef(false);
  const request = useMutation({
    mutationFn: () =>
      runSummary(ownerPubkey ?? "", activeCommunity?.relayUrl ?? ""),
    retry: false,
    onSuccess: onRequested,
  });
  return (
    <div className="space-y-2">
      <Button
        size="sm"
        disabled={!enabled || request.isPending}
        onClick={async () => {
          if (inFlight.current) return;
          inFlight.current = true;
          try {
            await request.mutateAsync();
          } catch {
            /* Error is rendered below. */
          } finally {
            inFlight.current = false;
          }
        }}
      >
        {request.isPending ? "Requesting…" : "Run summary"}
      </Button>
      {request.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {request.error.message} Check the agent conversation before retrying
          if delivery is uncertain.
        </p>
      ) : null}
      {request.isSuccess ? (
        <p role="status" className="text-sm text-muted-foreground">
          Summary requested.{" "}
          <Link
            to="/channels/$channelId"
            params={{ channelId: request.data.channelId }}
            className="underline"
          >
            View agent conversation
          </Link>
        </p>
      ) : null}
      {!enabled ? (
        <p className="text-xs text-muted-foreground">
          Choose an agent and at least one conversation in Summary settings.
        </p>
      ) : null}
    </div>
  );
}
