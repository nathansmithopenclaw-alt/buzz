import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { dispatchHistoryClient } from "../relayHistory";
import { readDispatchIssueSnapshot } from "../conversationUpdates";
import { buildDispatchLink, parseDispatchLink } from "../dispatchLink";
import { DispatchLinkAnchor } from "./DispatchLinkAnchor";
const watches = new Map<
  string,
  { users: number; dispose: Promise<() => void> }
>();
function watchChannel(scope: string, channel: string, changed: () => void) {
  let current = watches.get(scope);
  if (!current) {
    current = {
      users: 0,
      dispose: relayClient
        .subscribeLive(
          { kinds: [9], "#h": [channel], "#t": ["dispatch-update"], limit: 0 },
          changed,
        )
        .catch(() => () => {}),
    };
    watches.set(scope, current);
  }
  current.users++;
  return () => {
    if (--current.users === 0) {
      watches.delete(scope);
      void current.dispose.then((dispose) => dispose()).catch(() => {});
    }
  };
}
function WorkLink({
  workspace,
  issue,
  relay,
  signer,
}: {
  workspace: string;
  issue: string;
  relay: string;
  signer: string;
}) {
  const query = useQuery({
    queryKey: ["dispatch-conversation-issue", relay, signer, workspace, issue],
    queryFn: () =>
      readDispatchIssueSnapshot(workspace, issue, { relay, signer }),
    staleTime: 30000,
    refetchOnWindowFocus: true,
    refetchInterval: 60000,
    retry: false,
  });
  const current = query.data?.issues.find((item) => item.issueId === issue);
  const href = buildDispatchLink({ workspace, issue, relay });
  return (
    <span className="inline-flex max-w-full items-center gap-2 rounded border border-border bg-muted/40 px-2 py-1 text-xs">
      {query.isError ? (
        <>
          <DispatchLinkAnchor href={href}>
            Linked work unavailable
          </DispatchLinkAnchor>
          <button
            type="button"
            className="underline"
            onClick={() => void query.refetch()}
          >
            Retry
          </button>
        </>
      ) : (
        <DispatchLinkAnchor href={href}>
          {current
            ? `${current.identifier} · ${current.title} · ${current.status.replaceAll("_", " ")}`
            : query.isLoading
              ? "Loading linked work…"
              : "Open linked work"}
        </DispatchLinkAnchor>
      )}
    </span>
  );
}
/** One shared channel read/subscription supplies all visible source-message links.
 * Message metadata only discovers an issue; current labels come from its authorized fold. */
export function ConversationWorkLinks({
  channel,
  messageId,
}: {
  channel: string;
  messageId: string;
}) {
  const { activeCommunity } = useCommunities(),
    identity = useIdentityQuery(),
    client = useQueryClient();
  const relay = activeCommunity?.relayUrl ?? "",
    signer = identity.data?.pubkey ?? "";
  const queryKey = React.useMemo(
    () => ["dispatch-conversation-links", relay, signer, channel],
    [relay, signer, channel],
  );
  const query = useQuery({
    queryKey,
    queryFn: () =>
      dispatchHistoryClient().fetchExhaustively({
        kinds: [9],
        "#h": [channel],
        "#t": ["dispatch-update"],
      }),
    enabled: Boolean(relay && signer),
    staleTime: 30000,
    retry: false,
  });
  React.useEffect(() => {
    if (!relay || !signer) return;
    return watchChannel(`${relay}:${signer}:${channel}`, channel, () => {
      void client.invalidateQueries({ queryKey });
      void client.invalidateQueries({
        queryKey: ["dispatch-conversation-issue", relay, signer],
      });
    });
  }, [relay, signer, channel, client, queryKey]);
  const references = new Map<string, { workspace: string; issue: string }>();
  for (const event of query.data ?? []) {
    if (
      event.id !== messageId &&
      !event.tags.some((tag) => tag[0] === "q" && tag[1] === messageId)
    )
      continue;
    const descriptor = event.tags.find((tag) => tag[0] === "dispatch");
    if (descriptor?.length !== 5) continue;
    const [workspace, issue] = descriptor.slice(1, 3);
    const parsed = parseDispatchLink(
      `buzz://dispatch?${new URLSearchParams({ workspace, issue, relay })}`,
    );
    if (parsed) references.set(`${workspace}:${issue}`, { workspace, issue });
  }
  if (query.isError || !references.size) return null;
  return (
    <ul className="mt-2 flex flex-wrap gap-2" aria-label="Linked Dispatch work">
      {[...references].slice(0, 5).map(([key, value]) => (
        <li key={key}>
          <WorkLink {...value} relay={relay} signer={signer} />
        </li>
      ))}
    </ul>
  );
}
