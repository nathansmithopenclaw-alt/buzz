import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import type { AutomationControls } from "../recordTypes";

export function AutomationSummary({
  controls,
}: {
  controls: AutomationControls;
}) {
  const { channels } = useChannelNavigation();
  const profiles = useUsersBatchQuery(
    controls.routing.map((route) => route.executor).filter(Boolean),
  );
  const channelName = (id: string) => {
    const channel = channels.find((channel) => channel.id === id);
    return channel
      ? `#${channel.name}`
      : "a channel you no longer have access to";
  };
  const agentName = (key: string) =>
    resolveUserLabel({
      pubkey: key,
      profiles: profiles.data?.profiles,
      fallbackName: "the selected agent",
    });
  return (
    <div className="space-y-3 text-sm" data-testid="automation-rule-summary">
      {controls.capture_channels.length ? (
        <div>
          <p className="font-medium">When a message gets a 📌 reaction</p>
          <p className="mt-1 text-muted-foreground">
            In {controls.capture_channels.map(channelName).join(", ")}, Buzz
            creates an issue with the message and a link back.
          </p>
        </div>
      ) : null}
      {controls.routing.map((route) => (
        <div key={`${route.label}:${route.executor}:${route.channel}`}>
          <p className="font-medium">
            When selected work with “{route.label}” is ready
          </p>
          <p className="mt-1 text-muted-foreground">
            Send it to {agentName(route.executor)} in{" "}
            {channelName(route.channel)}. Its owner stays responsible for
            reviewing the result.
          </p>
        </div>
      ))}
      {controls.notify_triage ? (
        <div>
          <p className="font-medium">
            When selected work needs an owner in Triage
          </p>
          <p className="mt-1 text-muted-foreground">
            Post a reminder in {channelName(controls.notification_channel)}.
          </p>
        </div>
      ) : null}
      {controls.unassigned_after_secs !== null ? (
        <div>
          <p className="font-medium">
            When selected work has no owner after{" "}
            {controls.unassigned_after_secs / 3600} hours
          </p>
          <p className="mt-1 text-muted-foreground">
            Post a reminder in {channelName(controls.notification_channel)}.
          </p>
        </div>
      ) : null}
    </div>
  );
}
