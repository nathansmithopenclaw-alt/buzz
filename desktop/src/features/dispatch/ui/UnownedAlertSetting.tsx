import { RelaySettingRecovery } from "./RelaySettingRecovery";
import * as React from "react";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import { activeRecords, type UnownedAlertAuthorization } from "../recordTypes";
import { saveRelayAuthorization } from "../relayAutomation";
import type { DispatchContext } from "../types";

export function UnownedAlertSetting({
  workspace,
  context,
  refresh,
}: {
  workspace: string;
  context: DispatchContext;
  refresh: () => Promise<DispatchContext>;
}) {
  const { channels } = useChannelNavigation();
  const matches = activeRecords(context.records, "unowned_alert");
  const current = matches[0];
  const [base, setBase] = React.useState(current);
  const [draft, setDraft] = React.useState<UnownedAlertAuthorization>(
    () =>
      current?.record.payload ?? {
        type: "unowned_alert",
        workspace,
        channel: null,
        after_hours: 24,
        enabled: true,
      },
  );
  const [busy, setBusy] = React.useState(false);
  const [dirty, setDirty] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (!dirty && current) {
      setDraft(current.record.payload);
      setBase(current);
    }
  }, [current, dirty]);
  const change = (patch: Partial<UnownedAlertAuthorization>) => {
    setDraft((v) => ({ ...v, ...patch }));
    setDirty(true);
  };
  async function save() {
    setBusy(true);
    setError(null);
    try {
      if (
        !Number.isInteger(draft.after_hours) ||
        draft.after_hours < 1 ||
        draft.after_hours > 8760
      )
        throw new Error("Choose a threshold between 1 and 8760 hours.");
      const accepted = await saveRelayAuthorization({
        payload: draft,
        previous: base,
        refresh,
      });
      if (accepted?.record.payload.type === "unowned_alert") {
        setDraft(accepted.record.payload);
        setBase(accepted as typeof current);
      }
      setDirty(false);
      await refresh();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="space-y-4" data-testid="dispatch-unowned-alert">
      <fieldset
        disabled={
          busy ||
          !context.canAuthorizeAutomation ||
          Boolean(context.sectionErrors.records) ||
          matches.length > 1
        }
        className="space-y-4"
      >
        <label
          htmlFor="unowned-enabled"
          className="flex items-center justify-between gap-3 text-sm font-medium"
        >
          Alert when work has no owner
          <Switch
            id="unowned-enabled"
            checked={draft.enabled}
            onCheckedChange={(enabled) => change({ enabled })}
          />
        </label>
        <div className="flex flex-wrap items-end gap-3">
          <label htmlFor="unowned-hours" className="grid gap-1 text-sm">
            after
            <Input
              id="unowned-hours"
              className="w-24"
              type="number"
              min={1}
              max={8760}
              value={draft.after_hours}
              onChange={(e) => change({ after_hours: Number(e.target.value) })}
            />
          </label>
          <span className="pb-2 text-sm">hours</span>
          <label className="grid gap-1 text-sm">
            post in
            <select
              className="h-9 rounded-md border border-input bg-background px-3 text-sm"
              value={draft.channel ?? ""}
              onChange={(e) => change({ channel: e.target.value || null })}
            >
              <option value="">The issue's home channel</option>
              {channels.map((channel) => (
                <option key={channel.id} value={channel.id}>
                  {channel.name}
                </option>
              ))}
            </select>
          </label>
        </div>
        <p className="text-sm text-muted-foreground">
          Buzz posts one reminder per channel listing issues that still need an
          owner. It never assigns work by itself.
        </p>
        {!current ? (
          <p className="text-xs text-muted-foreground">
            Alerts start when you save these settings.
          </p>
        ) : null}
        <Button
          disabled={busy || (Boolean(current) && !dirty && !error)}
          onClick={() => void save()}
        >
          {busy ? "Saving…" : "Save alert settings"}
        </Button>
      </fieldset>
      {error || matches.length > 1 ? (
        <p role="alert" className="text-sm text-destructive">
          {error ?? "Multiple alert records exist. Resolve them before saving."}
        </p>
      ) : null}
      <RelaySettingRecovery
        onRecovered={() => {
          setDirty(false);
          setError(null);
        }}
        stream={workspace}
        setting={`unowned_alert:${workspace}`}
        refresh={refresh}
      />
    </section>
  );
}
