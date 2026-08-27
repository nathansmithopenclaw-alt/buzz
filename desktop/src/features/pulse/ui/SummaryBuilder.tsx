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

type SummaryBuilderProps = {
  conversations: SummaryConversation[];
  sourceConfig: SummarySourceConfig;
  hour: number;
  minute: number;
  timezone: string;
  reporterPubkey: string | null;
  agents: ManagedAgent[];
  saving: boolean;
  error: string | null;
  onSourceConfigChange: (config: SummarySourceConfig) => void;
  onTimeChange: (hour: number, minute: number) => void;
  onReporterChange: (pubkey: string | null) => void;
  onSave: () => void;
  onCancel?: () => void;
};

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
  reporterPubkey,
  agents,
  saving,
  error,
  onSourceConfigChange,
  onTimeChange,
  onReporterChange,
  onSave,
  onCancel,
}: SummaryBuilderProps) {
  const { channels, dms } = partitionLiveConversations(conversations);
  const timeValue = formatLocalTime(hour, minute);

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

      {agents.length > 0 ? (
        <SettingsOptionGroup
          description="v1 adds the reporter as a bot on included channels. Direct messages stay on the read-grant path."
          title="Reporter"
        >
          <SettingsOptionRow>
            <label
              className="min-w-0 flex-1 text-sm font-medium"
              htmlFor="summary-reporter"
            >
              Colleague
            </label>
            <select
              className="h-8 max-w-56 rounded-md border border-border/70 bg-background px-2 text-sm"
              id="summary-reporter"
              onChange={(event) =>
                onReporterChange(
                  event.target.value === "" ? null : event.target.value,
                )
              }
              value={reporterPubkey ?? ""}
            >
              <option value="">None yet</option>
              {agents.map((agent) => (
                <option key={agent.pubkey} value={agent.pubkey}>
                  {agent.name}
                </option>
              ))}
            </select>
          </SettingsOptionRow>
        </SettingsOptionGroup>
      ) : null}

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

      <SettingsOptionGroup title="Direct messages">
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
        <Button disabled={saving} onClick={onSave} type="button">
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
