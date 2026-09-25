import * as React from "react";
import { toast } from "sonner";

import {
  listConnections,
  type ModelConnection,
} from "@/features/model-connections/api";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import {
  listSubscriptionAccounts,
  type SubscriptionAccount,
} from "@/shared/api/tauriSubscriptionAccounts";
import { useFeatureEnabled } from "@/shared/features";
import { isMacPlatform } from "@/shared/lib/platform";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import {
  type DictationCleanupSource,
  type DictationModelInfo,
  type DictationSettings,
  downloadDictationModel,
  getDictationCleanupSources,
  getDictationModelInfo,
} from "../api";
import {
  DICTATION_DEVICE_STORAGE_KEY,
  listAudioInputs,
} from "../lib/audioCapture";
import { useDictationSettings } from "../useDictationSettings";
import { hotkeyLabel } from "./DictationButton";

/** Recommended cleanup models (from the Buzz dictation benchmark). */
export const RECOMMENDED_CONNECTION_MODEL = "google/gemini-3.8-flash";
export const RECOMMENDED_CLAUDE_PLAN_MODEL = "claude-haiku-4-5";

const SELECT_CLASS =
  "h-7 rounded-md border border-input bg-background px-2 text-xs shadow-xs focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring";

const gb = (bytes: number) => `${(bytes / 1e9).toFixed(1)} GB`;

function modelStatusText(info: DictationModelInfo | null): string {
  if (!info) return "Checking…";
  const { status } = info;
  if (status === "ready")
    return "Ready — Parakeet 0.6B v2 (English), on this Mac";
  if (status === "not_downloaded") return "Not downloaded (482 MB)";
  if ("downloading" in status)
    return `Downloading… ${status.downloading.progress_percent}%`;
  return `Download failed: ${status.error}`;
}

function useModelInfo(enabled: boolean) {
  const [info, setInfo] = React.useState<DictationModelInfo | null>(null);
  const refresh = React.useCallback(() => {
    void getDictationModelInfo()
      .then(setInfo)
      .catch(() => setInfo(null));
  }, []);
  React.useEffect(() => {
    if (enabled) refresh();
  }, [enabled, refresh]);
  const downloading =
    info !== null &&
    typeof info.status === "object" &&
    "downloading" in info.status;
  React.useEffect(() => {
    if (!downloading) return;
    const timer = window.setInterval(refresh, 1500);
    return () => window.clearInterval(timer);
  }, [downloading, refresh]);
  return { info, refresh };
}

function readDevice(): string {
  try {
    return window.localStorage.getItem(DICTATION_DEVICE_STORAGE_KEY) ?? "";
  } catch {
    return "";
  }
}

/**
 * The dictation microphone: a per-device convenience kept in local storage
 * ("" = system default). The list refreshes when devices come and go.
 */
function useMicrophoneChoice(enabled: boolean) {
  const [inputs, setInputs] = React.useState<
    { deviceId: string; label: string }[]
  >([]);
  const [deviceId, setDeviceId] = React.useState(readDevice);
  React.useEffect(() => {
    if (!enabled) return;
    const refresh = () =>
      void listAudioInputs()
        .then(setInputs)
        .catch(() => setInputs([]));
    refresh();
    navigator.mediaDevices?.addEventListener?.("devicechange", refresh);
    return () =>
      navigator.mediaDevices?.removeEventListener?.("devicechange", refresh);
  }, [enabled]);
  const choose = (next: string) => {
    setDeviceId(next);
    try {
      if (next) window.localStorage.setItem(DICTATION_DEVICE_STORAGE_KEY, next);
      else window.localStorage.removeItem(DICTATION_DEVICE_STORAGE_KEY);
    } catch {
      toast.error("Buzz could not save your microphone choice.");
    }
  };
  return { inputs, deviceId, choose };
}

/**
 * Stage 1 dictation settings (shown under Experiments while the dictation
 * experiment is on): speech model download, cleanup source, timeout and the
 * cloud-cleanup switch. The full setup page replaces this in Stage 2.
 */
export function DictationSettingsSection() {
  const enabled = useFeatureEnabled("dictation");
  const { settings, save } = useDictationSettings(enabled);
  const { info, refresh } = useModelInfo(enabled);
  const [connections, setConnections] = React.useState<ModelConnection[]>([]);
  const [accounts, setAccounts] = React.useState<SubscriptionAccount[]>([]);
  const [agentOpenRouter, setAgentOpenRouter] = React.useState(false);
  const microphone = useMicrophoneChoice(enabled);

  React.useEffect(() => {
    if (!enabled) return;
    void listConnections()
      .then(setConnections)
      .catch(() => setConnections([]));
    void getDictationCleanupSources()
      .then((sources) => setAgentOpenRouter(sources.agentOpenRouter))
      .catch(() => setAgentOpenRouter(false));
    void listSubscriptionAccounts()
      .then((all) =>
        setAccounts(
          all.filter(
            (a) =>
              a.harness === "claude" &&
              a.kind === "token" &&
              a.state === "ready",
          ),
        ),
      )
      .catch(() => setAccounts([]));
  }, [enabled]);

  if (!enabled || !settings) return null;

  const update = (patch: Partial<DictationSettings>) => {
    void save({ ...settings, ...patch }).catch((error: unknown) =>
      toast.error(error instanceof Error ? error.message : String(error)),
    );
  };
  const setCleanup = (cleanup: DictationCleanupSource) => update({ cleanup });

  const onSourceChange = (kind: string) => {
    if (kind === "off") setCleanup({ kind: "off" });
    else if (kind === "agent_open_router") {
      setCleanup({
        kind: "agent_open_router",
        model: RECOMMENDED_CONNECTION_MODEL,
      });
    } else if (kind === "connection") {
      const first = connections[0];
      if (!first) {
        toast.info(
          "Add a model connection (e.g. OpenRouter) in Settings → Agents first.",
        );
        return;
      }
      setCleanup({
        kind: "connection",
        connectionId: first.id,
        model: RECOMMENDED_CONNECTION_MODEL,
      });
    } else {
      setCleanup({
        kind: "claude_plan",
        accountId: null,
        model: RECOMMENDED_CLAUDE_PLAN_MODEL,
      });
    }
  };

  const cleanup = settings.cleanup;
  const download = () =>
    void downloadDictationModel()
      .then(refresh)
      .catch((error: unknown) =>
        toast.error(error instanceof Error ? error.message : String(error)),
      );

  return (
    <SettingsOptionGroup
      title="Dictation"
      description="Speak into a composer with the sound-wave button or the dictation key. Audio never leaves this Mac."
    >
      <SettingsOptionRow data-testid="dictation-model-row">
        <div className="min-w-0">
          <div className="font-medium">Speech model</div>
          <div className="text-xs text-muted-foreground">
            {modelStatusText(info)}
            {info && info.status !== "ready" && info.freeBytes !== null
              ? ` · ${gb(info.freeBytes)} free, needs ${gb(info.requiredBytes)}`
              : ""}
          </div>
        </div>
        {info &&
        (info.status === "not_downloaded" ||
          (typeof info.status === "object" && "error" in info.status)) ? (
          <Button
            disabled={!info.enoughSpace}
            onClick={download}
            size="sm"
            type="button"
          >
            {info.enoughSpace ? "Download" : "Not enough space"}
          </Button>
        ) : null}
      </SettingsOptionRow>

      <SettingsOptionRow>
        <div className="min-w-0">
          <div className="font-medium">Microphone</div>
          <div className="text-xs text-muted-foreground">
            Falls back to the system default if this one is unplugged.
          </div>
        </div>
        <select
          aria-label="Dictation microphone"
          className={SELECT_CLASS}
          onChange={(e) => microphone.choose(e.target.value)}
          value={
            microphone.inputs.some((i) => i.deviceId === microphone.deviceId)
              ? microphone.deviceId
              : ""
          }
        >
          <option value="">System default</option>
          {microphone.inputs.map((input) => (
            <option key={input.deviceId} value={input.deviceId}>
              {input.label}
            </option>
          ))}
        </select>
      </SettingsOptionRow>

      <SettingsOptionRow>
        <div className="min-w-0">
          <div className="font-medium">Cleanup</div>
          <div className="text-xs text-muted-foreground">
            Removes filler words and turns spoken names into mentions.
            Recommended: Gemini 3.8 Flash through OpenRouter.
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <select
            aria-label="Cleanup source"
            className={SELECT_CLASS}
            onChange={(e) => onSourceChange(e.target.value)}
            value={cleanup.kind}
          >
            <option value="off">Off (as spoken)</option>
            {agentOpenRouter || cleanup.kind === "agent_open_router" ? (
              <option value="agent_open_router">
                OpenRouter (key from Agent settings)
              </option>
            ) : null}
            <option value="connection">Model connection</option>
            <option value="claude_plan">My Claude plan (personal use)</option>
          </select>
          {cleanup.kind === "connection" ? (
            <>
              <select
                aria-label="Cleanup connection"
                className={SELECT_CLASS}
                onChange={(e) =>
                  setCleanup({ ...cleanup, connectionId: e.target.value })
                }
                value={cleanup.connectionId}
              >
                {connections.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
              </select>
              <Input
                aria-label="Cleanup model"
                className="h-7 w-56 text-xs"
                defaultValue={cleanup.model}
                key={`${cleanup.connectionId}:${cleanup.model}`}
                onBlur={(e) => {
                  const model = e.target.value.trim();
                  if (model && model !== cleanup.model)
                    setCleanup({ ...cleanup, model });
                }}
              />
            </>
          ) : null}
          {cleanup.kind === "agent_open_router" ? (
            <Input
              aria-label="Cleanup model"
              className="h-7 w-56 text-xs"
              defaultValue={cleanup.model}
              key={`agent-openrouter:${cleanup.model}`}
              onBlur={(e) => {
                const model = e.target.value.trim();
                if (model && model !== cleanup.model)
                  setCleanup({ ...cleanup, model });
              }}
            />
          ) : null}
          {cleanup.kind === "claude_plan" ? (
            <select
              aria-label="Claude account"
              className={SELECT_CLASS}
              onChange={(e) =>
                setCleanup({ ...cleanup, accountId: e.target.value || null })
              }
              value={cleanup.accountId ?? ""}
            >
              <option value="">Default Claude account</option>
              {accounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.nickname ?? a.id}
                </option>
              ))}
            </select>
          ) : null}
        </div>
      </SettingsOptionRow>

      <SettingsOptionRow>
        <div className="min-w-0">
          <div className="font-medium">Cleanup timeout</div>
          <div className="text-xs text-muted-foreground">
            Past this, you get your words as spoken with mention suggestions.
          </div>
        </div>
        <select
          aria-label="Cleanup timeout"
          className={SELECT_CLASS}
          onChange={(e) => update({ timeoutMs: Number(e.target.value) })}
          value={settings.timeoutMs}
        >
          {[2000, 3000, 4000, 6000, 8000].map((ms) => (
            <option key={ms} value={ms}>
              {ms / 1000} seconds
            </option>
          ))}
        </select>
      </SettingsOptionRow>

      <SettingsOptionRow>
        <div className="min-w-0">
          <div className="font-medium">Cloud cleanup</div>
          <div className="text-xs text-muted-foreground">
            Sends only your words and channel member names. A secret check runs
            first.
          </div>
        </div>
        <Switch
          aria-label="Cloud cleanup"
          checked={settings.cloudCleanupEnabled}
          onCheckedChange={(checked) =>
            update({ cloudCleanupEnabled: checked })
          }
        />
      </SettingsOptionRow>

      <SettingsOptionRow>
        <div className="min-w-0">
          <div className="font-medium">Dictation key</div>
          <div className="text-xs text-muted-foreground">
            Tap to start and stop, or hold to talk. Esc cancels.
          </div>
        </div>
        <span className="text-xs">
          {hotkeyLabel(settings.hotkey, isMacPlatform())}
        </span>
      </SettingsOptionRow>
    </SettingsOptionGroup>
  );
}
