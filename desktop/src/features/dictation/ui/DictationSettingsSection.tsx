import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { AgentDropdownSelect } from "@/features/agents/ui/agentConfigControls";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { useFeatureEnabled } from "@/shared/features";
import { Switch } from "@/shared/ui/switch";
import { getCleanupCatalog, type DictationSettings } from "../api";
import { useDictationSettings } from "../useDictationSettings";
import { DictationSpeechRow } from "./DictationSpeechRow";
import { DictationMicrophoneRow } from "./DictationMicrophoneRow";
import { DictationCleanupRow } from "./DictationCleanupRow";
import { DictationKeyControl } from "./DictationKeyControl";

/** Device-local dictation controls share Voice's existing settings groups. */
export function DictationSettingsSection() {
  const enabled = useFeatureEnabled("dictation");
  const { settings, save, isLoading, refetch } = useDictationSettings(enabled);
  const catalog = useQuery({
    queryKey: ["dictation-cleanup-catalog"],
    queryFn: getCleanupCatalog,
    enabled,
  });
  const [saving, setSaving] = React.useState(false);
  const update = (patch: Partial<DictationSettings>) => {
    if (!settings || saving) return;
    setSaving(true);
    void save({ ...settings, ...patch })
      .catch((error) => toast.error(String(error)))
      .finally(() => setSaving(false));
  };
  if (!enabled) return null;
  if (!settings)
    return (
      <SettingsOptionGroup title="Dictation">
        <SettingsOptionRow>
          <p role="status">
            {isLoading
              ? "Loading dictation settings…"
              : "Could not load dictation settings"}
            {!isLoading ? (
              <button
                type="button"
                className="ml-2 underline"
                onClick={() => void refetch()}
              >
                Retry
              </button>
            ) : null}
          </p>
        </SettingsOptionRow>
      </SettingsOptionGroup>
    );
  const cleanupOn = settings.cleanup.kind !== "off";
  const local =
    settings.cleanup.kind === "connection" &&
    catalog.data?.localConnectionIds.includes(settings.cleanup.connectionId);
  return (
    <SettingsOptionGroup title="Dictation" data-testid="dictation-settings">
      <DictationSpeechRow
        modelId={settings.speech?.modelId ?? "parakeet-tdt-0.6b-v2-int8"}
        disabled={saving}
        onChange={(modelId) => update({ speech: { kind: "local", modelId } })}
      />
      <DictationMicrophoneRow />
      <DictationCleanupRow
        value={settings.cleanup}
        disabled={saving}
        catalog={catalog.data}
        onRetryCatalog={() => void catalog.refetch()}
        onChange={(cleanup) => update({ cleanup })}
      />
      {catalog.error ? (
        <SettingsOptionRow>
          <p role="alert">
            Could not load cleanup sources.{" "}
            <button
              type="button"
              className="underline"
              onClick={() => void catalog.refetch()}
            >
              Retry
            </button>
          </p>
        </SettingsOptionRow>
      ) : null}
      {cleanupOn ? (
        <SettingsOptionRow>
          <label className="font-medium" htmlFor="dictation-cleanup-timeout">
            Cleanup timeout
          </label>
          <AgentDropdownSelect
            className="w-80 max-w-full"
            id="dictation-cleanup-timeout"
            disabled={saving}
            value={String(settings.timeoutMs)}
            onValueChange={(value) => update({ timeoutMs: Number(value) })}
            options={[
              ...new Set([2000, 3000, 4000, 6000, 8000, settings.timeoutMs]),
            ]
              .sort((a, b) => a - b)
              .map((value) => ({
                value: String(value),
                label: `${value / 1000} seconds`,
              }))}
          />
        </SettingsOptionRow>
      ) : null}
      {cleanupOn && !local ? (
        <SettingsOptionRow>
          <div>
            <label className="font-medium" htmlFor="dictation-cloud-cleanup">
              Cloud cleanup
            </label>
            <p className="text-xs text-muted-foreground">
              Send words and member names for cleanup.
            </p>
          </div>
          <Switch
            id="dictation-cloud-cleanup"
            checked={settings.cloudCleanupEnabled}
            disabled={saving}
            onCheckedChange={(cloudCleanupEnabled) =>
              update({ cloudCleanupEnabled })
            }
          />
        </SettingsOptionRow>
      ) : null}
      <SettingsOptionRow>
        <div>
          <p className="font-medium">Dictation key</p>
          <p className="text-xs text-muted-foreground">
            Tap to start and stop, or hold to talk. Esc cancels.
          </p>
        </div>
        <DictationKeyControl
          value={settings.hotkey}
          disabled={saving}
          onChange={(hotkey) => update({ hotkey })}
        />
      </SettingsOptionRow>
    </SettingsOptionGroup>
  );
}
