import {
  NECTAR_AVATAR_PATH,
  NECTAR_DESCRIPTION,
  NECTAR_DISPLAY_NAME,
  NECTAR_PERSONA_ID,
} from "@/features/pulse/lib/nectarPersona";
import {
  formatLocalTime,
  isConversationChecked,
  parseLocalTime,
  partitionLiveConversations,
  toggleConversation,
  type SummaryConversation,
  type SummarySourceConfig,
} from "@/features/pulse/lib/summarySources";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import type { ManagedAgent } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";

export type SummaryReporterMode = "nectar" | "existing";

export type SummaryRuntimeOption = {
  id: string;
  label: string;
};

export type SummaryModelOption = {
  id: string;
  label: string;
};

type SummaryBuilderProps = {
  conversations: SummaryConversation[];
  sourceConfig: SummarySourceConfig;
  hour: number;
  minute: number;
  timezone: string;
  reporterMode: SummaryReporterMode;
  reporterPubkey: string | null;
  agents: ManagedAgent[];
  nectarExists: boolean;
  runtimeId: string;
  runtimes: SummaryRuntimeOption[];
  runtimesLoading: boolean;
  model: string;
  modelOptions: SummaryModelOption[];
  modelLoading: boolean;
  saving: boolean;
  error: string | null;
  onSourceConfigChange: (config: SummarySourceConfig) => void;
  onTimeChange: (hour: number, minute: number) => void;
  onReporterModeChange: (mode: SummaryReporterMode) => void;
  onReporterChange: (pubkey: string | null) => void;
  onRuntimeChange: (runtimeId: string) => void;
  onModelChange: (model: string) => void;
  onSave: () => void;
  onCancel?: () => void;
};

const NECTAR_SELECT_VALUE = "nectar";
const selectClassName =
  "h-8 max-w-56 rounded-md border border-border/70 bg-background px-2 text-sm";

function ConversationRow({
  conversation,
  checked,
  onCheckedChange,
}: {
  conversation: SummaryConversation;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  const switchId = `summary-source-${conversation.id}`;
  return (
    <SettingsOptionRow>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium" id={`${switchId}-label`}>
          {conversation.kind === "channel"
            ? `#${conversation.name}`
            : conversation.name}
        </p>
      </div>
      <Switch
        aria-labelledby={`${switchId}-label`}
        checked={checked}
        data-testid={switchId}
        onCheckedChange={onCheckedChange}
      />
    </SettingsOptionRow>
  );
}

export function SummaryBuilder({
  conversations,
  sourceConfig,
  hour,
  minute,
  timezone,
  reporterMode,
  reporterPubkey,
  agents,
  nectarExists,
  runtimeId,
  runtimes,
  runtimesLoading,
  model,
  modelOptions,
  modelLoading,
  saving,
  error,
  onSourceConfigChange,
  onTimeChange,
  onReporterModeChange,
  onReporterChange,
  onRuntimeChange,
  onModelChange,
  onSave,
  onCancel,
}: SummaryBuilderProps) {
  const { channels, dms } = partitionLiveConversations(conversations);
  const timeValue = formatLocalTime(hour, minute);
  const otherAgents = agents.filter(
    (agent) => agent.personaId !== NECTAR_PERSONA_ID,
  );
  const reporterSelectValue =
    reporterMode === "nectar" ? NECTAR_SELECT_VALUE : (reporterPubkey ?? "");
  const showNectarSetup = reporterMode === "nectar" && !nectarExists;

  return (
    <div className="flex flex-col gap-8">
      <div>
        <h2 className="text-lg font-semibold tracking-tight">
          Build your Summary
        </h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Choose the conversations you belong to. New rooms you join can be
          included automatically. Archived and deleted rooms drop off on their
          own.
        </p>
      </div>

      <SettingsOptionGroup
        description="09:00 means 09:00 where you are, including daylight saving."
        title="Schedule"
      >
        <SettingsOptionRow>
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium" id="summary-time-label">
              Daily time
            </p>
            <p
              className="text-xs text-muted-foreground/70"
              data-settings-subcopy
            >
              {timezone}
            </p>
          </div>
          <Input
            aria-labelledby="summary-time-label"
            className="w-32"
            onChange={(event) => {
              const parsed = parseLocalTime(event.target.value);
              if (parsed) onTimeChange(parsed.hour, parsed.minute);
            }}
            type="time"
            value={timeValue}
          />
        </SettingsOptionRow>
        <SettingsOptionRow>
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium" id="summary-auto-include-label">
              Automatically include new conversations
            </p>
            <p
              className="text-xs text-muted-foreground/70"
              data-settings-subcopy
            >
              Only rooms you join. Not the whole community catalog.
            </p>
          </div>
          <Switch
            aria-labelledby="summary-auto-include-label"
            checked={sourceConfig.autoInclude}
            data-testid="summary-auto-include"
            onCheckedChange={(autoInclude) =>
              onSourceConfigChange({ ...sourceConfig, autoInclude })
            }
          />
        </SettingsOptionRow>
      </SettingsOptionGroup>

      <SettingsOptionGroup
        description="Nectar is the supplied Summary colleague. Choose a harness and model — name, avatar, and instructions are already written. Save adds the reporter as a bot on included channels and creates the daily run."
        title="Reporter"
      >
        {reporterMode === "nectar" ? (
          <SettingsOptionRow data-testid="summary-nectar">
            <div className="flex min-w-0 flex-1 items-center gap-3">
              <img
                alt=""
                className="h-12 w-12 shrink-0 rounded-full bg-white object-cover"
                src={NECTAR_AVATAR_PATH}
              />
              <div className="min-w-0">
                <p className="truncate text-sm font-medium">
                  {NECTAR_DISPLAY_NAME}
                </p>
                <p
                  className="text-xs text-muted-foreground/70"
                  data-settings-subcopy
                >
                  {nectarExists
                    ? "Already in My Agents. Save assigns the Summary job."
                    : NECTAR_DESCRIPTION}
                </p>
              </div>
            </div>
          </SettingsOptionRow>
        ) : null}
        {otherAgents.length > 0 ? (
          <SettingsOptionRow>
            <label
              className="min-w-0 flex-1 text-sm font-medium"
              htmlFor="summary-reporter"
            >
              Colleague
            </label>
            <select
              className={selectClassName}
              data-testid="summary-reporter"
              id="summary-reporter"
              onChange={(event) => {
                const value = event.target.value;
                if (value === NECTAR_SELECT_VALUE) {
                  onReporterModeChange("nectar");
                  onReporterChange(null);
                  return;
                }
                onReporterModeChange("existing");
                onReporterChange(value === "" ? null : value);
              }}
              value={reporterSelectValue}
            >
              <option value={NECTAR_SELECT_VALUE}>{NECTAR_DISPLAY_NAME}</option>
              {otherAgents.map((agent) => (
                <option key={agent.pubkey} value={agent.pubkey}>
                  {agent.name}
                </option>
              ))}
            </select>
          </SettingsOptionRow>
        ) : null}
        {showNectarSetup ? (
          <>
            <SettingsOptionRow>
              <label
                className="min-w-0 flex-1 text-sm font-medium"
                htmlFor="summary-nectar-harness"
              >
                Harness
              </label>
              <select
                className={selectClassName}
                data-testid="summary-nectar-harness"
                disabled={runtimesLoading || runtimes.length === 0}
                id="summary-nectar-harness"
                onChange={(event) => onRuntimeChange(event.target.value)}
                value={runtimeId}
              >
                <option value="">
                  {runtimesLoading ? "Loading harnesses…" : "Choose a harness"}
                </option>
                {runtimes.map((runtime) => (
                  <option key={runtime.id} value={runtime.id}>
                    {runtime.label}
                  </option>
                ))}
              </select>
            </SettingsOptionRow>
            <SettingsOptionRow>
              <label
                className="min-w-0 flex-1 text-sm font-medium"
                htmlFor="summary-nectar-model"
              >
                Model
              </label>
              <select
                className={selectClassName}
                data-testid="summary-nectar-model"
                disabled={!runtimeId || modelLoading}
                id="summary-nectar-model"
                onChange={(event) => onModelChange(event.target.value)}
                value={model}
              >
                <option value="">
                  {modelLoading ? "Loading models…" : "Default model"}
                </option>
                {modelOptions.map((option) =>
                  option.id ? (
                    <option key={option.id} value={option.id}>
                      {option.label}
                    </option>
                  ) : null,
                )}
              </select>
            </SettingsOptionRow>
          </>
        ) : null}
      </SettingsOptionGroup>

      <SettingsOptionGroup title="Channels">
        {channels.length === 0 ? (
          <SettingsOptionRow>
            <p className="text-sm text-muted-foreground">
              You are not a member of any live channels.
            </p>
          </SettingsOptionRow>
        ) : (
          channels.map((conversation) => (
            <ConversationRow
              checked={isConversationChecked(sourceConfig, conversation)}
              conversation={conversation}
              key={conversation.id}
              onCheckedChange={(checked) =>
                onSourceConfigChange(
                  toggleConversation(sourceConfig, conversation, checked),
                )
              }
            />
          ))
        )}
      </SettingsOptionGroup>

      <SettingsOptionGroup
        description="Listed so you can choose them now. v1 does not grant the reporter access to DMs — that needs a native read-grant that does not exist yet."
        title="Direct messages"
      >
        {dms.length === 0 ? (
          <SettingsOptionRow>
            <p className="text-sm text-muted-foreground">
              No direct messages yet.
            </p>
          </SettingsOptionRow>
        ) : (
          dms.map((conversation) => (
            <ConversationRow
              checked={isConversationChecked(sourceConfig, conversation)}
              conversation={conversation}
              key={conversation.id}
              onCheckedChange={(checked) =>
                onSourceConfigChange(
                  toggleConversation(sourceConfig, conversation, checked),
                )
              }
            />
          ))
        )}
      </SettingsOptionGroup>

      {error ? (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}

      <div className="flex items-center gap-2">
        <Button
          disabled={
            saving ||
            (showNectarSetup && (runtimesLoading || runtimeId.length === 0))
          }
          onClick={onSave}
          type="button"
        >
          {saving ? "Saving…" : "Save"}
        </Button>
        {onCancel ? (
          <Button
            disabled={saving}
            onClick={onCancel}
            type="button"
            variant="ghost"
          >
            Cancel
          </Button>
        ) : null}
      </div>
    </div>
  );
}
