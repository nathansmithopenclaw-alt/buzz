import * as React from "react";
import { stringify } from "yaml";
import { useCommunities } from "@/features/communities/useCommunities";
import { useChannelWorkflowsQuery } from "@/features/workflows/hooks";
import { createWorkflow, updateWorkflow } from "@/shared/api/tauriWorkflows";
import { Switch } from "@/shared/ui/switch";
import { captureWorkflowDefinition } from "../captureWorkflow";

export function ChannelCaptureSetting({
  channel,
  signer,
}: {
  channel: string;
  signer: string;
}) {
  const { activeCommunity } = useCommunities();
  const workflows = useChannelWorkflowsQuery(channel);
  const matches =
    workflows.data?.filter((value) => value.name === "dispatch-capture") ?? [];
  const workflow = matches[0];
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const mounted = React.useRef(true);
  React.useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  async function save(enabled: boolean) {
    if (!activeCommunity || busy || matches.length > 1) return;
    setBusy(true);
    setError(null);
    try {
      const definition = stringify(captureWorkflowDefinition(channel, enabled));
      const scope = {
        expectedRelayUrl: activeCommunity.relayUrl,
        expectedSignerPubkey: signer,
      };
      if (workflow)
        await updateWorkflow(workflow.id, definition, workflow.revision, scope);
      else await createWorkflow(channel, definition, scope);
      await workflows.refetch();
    } catch (cause) {
      if (mounted.current) setError(String(cause));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  return (
    <section className="space-y-2" data-testid="channel-capture-setting">
      <div className="flex items-center justify-between gap-3">
        <label htmlFor="dispatch-pin-capture" className="text-sm font-medium">
          Pin reactions create Dispatch issues
        </label>
        <Switch
          id="dispatch-pin-capture"
          checked={workflow?.definition.enabled !== false && Boolean(workflow)}
          disabled={
            busy ||
            workflows.isLoading ||
            Boolean(workflows.error) ||
            matches.length > 1
          }
          onCheckedChange={(value) => void save(value)}
          aria-describedby="dispatch-pin-capture-help"
        />
      </div>
      <p
        id="dispatch-pin-capture-help"
        className="text-sm text-muted-foreground"
      >
        When someone adds 📌 to a message here, Buzz creates an issue in Triage
        with a link back to the message.
      </p>
      {error || workflows.error || matches.length > 1 ? (
        <p role="alert" className="text-sm text-destructive">
          {error ??
            (matches.length > 1
              ? "Multiple capture workflows exist. Resolve them in Workflows before changing this setting."
              : String(workflows.error))}
        </p>
      ) : null}
    </section>
  );
}
