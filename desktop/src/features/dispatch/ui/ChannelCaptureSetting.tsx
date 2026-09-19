import { RelaySettingRecovery } from "./RelaySettingRecovery";
import * as React from "react";
import { useChannelWorkflowsQuery } from "@/features/workflows/hooks";
import { Switch } from "@/shared/ui/switch";
import { Button } from "@/shared/ui/button";
import { useDispatchContext, useDispatchWorkspaces } from "../hooks";
import { activeRecords } from "../recordTypes";
import { saveRelayAuthorization } from "../relayAutomation";

export function ChannelCaptureSetting({ channel }: { channel: string }) {
  const workflows = useChannelWorkflowsQuery(channel);
  const context = useDispatchContext(channel);
  const discovery = useDispatchWorkspaces();
  const grants = activeRecords(context.records, "capture").filter(
    (value) => value.record.payload.channel === channel,
  );
  const current = grants[0];
  const matches =
    workflows.data?.filter((value) => value.name === "dispatch-capture") ?? [];
  const populated = discovery.workspaces.filter(
    (value) => value.trackerEventCount > 0,
  );
  const [selected, setSelected] = React.useState<string | null>(null);
  const workspace =
    selected ??
    current?.record.payload.workspace ??
    (populated.length === 1 ? populated[0].id : "");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const retryIntent = React.useRef(false);
  const enabled = Boolean(
    current?.record.payload.enabled &&
      matches[0]?.definition.enabled !== false &&
      matches.length,
  );
  async function save(next: boolean) {
    if (busy) return;
    retryIntent.current = next;
    setBusy(true);
    setError(null);
    try {
      if (!workspace)
        throw new Error(
          "Choose the workspace where captured issues should go.",
        );
      await saveRelayAuthorization({
        payload: {
          type: "capture",
          channel,
          workspace,
          project: current?.record.payload.project ?? null,
          enabled: next,
        },
        previous: current,
        refresh: context.refresh,
      });
      await Promise.all([context.refresh(), workflows.refetch()]);
      setSelected(null);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }
  const disabled =
    busy ||
    context.isLoading ||
    context.isPreview ||
    Boolean(
      context.error ||
        context.sectionErrors.records ||
        workflows.error ||
        discovery.error,
    ) ||
    !context.canAuthorizeAutomation ||
    workflows.isLoading ||
    matches.length > 1 ||
    grants.length > 1;
  return (
    <section className="space-y-2" data-testid="channel-capture-setting">
      <div className="flex items-center justify-between gap-3">
        <label htmlFor="dispatch-pin-capture" className="text-sm font-medium">
          Pin reactions create Dispatch issues
        </label>
        <Switch
          id="dispatch-pin-capture"
          checked={enabled}
          disabled={disabled || !workspace}
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
      <label className="flex items-center gap-2 text-sm">
        Issues go to
        <select
          disabled={disabled}
          className="h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-2 text-sm"
          value={workspace}
          onChange={(e) => setSelected(e.target.value)}
        >
          <option value="">Choose workspace</option>
          {discovery.workspaces.map((value) => (
            <option key={value.id} value={value.id}>
              {value.name}
            </option>
          ))}
        </select>
      </label>
      {selected && current ? (
        <Button
          size="sm"
          disabled={disabled}
          onClick={() => void save(enabled)}
        >
          Save capture setting
        </Button>
      ) : null}
      {error ||
      context.error ||
      workflows.error ||
      discovery.error ||
      grants.length > 1 ||
      matches.length > 1 ? (
        <p role="alert" className="text-sm text-destructive">
          {error ??
            (grants.length > 1 || matches.length > 1
              ? "Multiple capture settings exist. Resolve them before saving."
              : String(context.error ?? workflows.error ?? discovery.error))}
          {error ? (
            <Button
              size="sm"
              variant="outline"
              disabled={disabled}
              onClick={() => void save(retryIntent.current)}
            >
              Retry save
            </Button>
          ) : null}
        </p>
      ) : null}
      <RelaySettingRecovery
        onRecovered={() => {
          setSelected(null);
          setError(null);
        }}
        stream={channel}
        setting={`capture:${channel}`}
        refresh={context.refresh}
      />
    </section>
  );
}
