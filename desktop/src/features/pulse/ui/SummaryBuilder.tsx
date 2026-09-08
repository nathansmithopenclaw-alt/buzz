import { AgentDialog } from "@/features/agents/ui/AgentDialog";
import * as React from "react";
import { UsersRound } from "lucide-react";
import { ManagedAgentPickerDialog } from "@/features/agents/ui/ManagedAgentPickerDialog";
import {
  SummaryRuntimeFields,
  type SummaryRuntimeDraft,
} from "./SummaryRuntimeFields";
import {
  NECTAR_AVATAR_PATH,
  NECTAR_DESCRIPTION,
  NECTAR_DISPLAY_NAME,
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
  onRuntimeDraftChange: (draft: SummaryRuntimeDraft | null) => void;
  runtimeValid: boolean;
  saving: boolean;
  error: string | null;
  onSourceConfigChange: (config: SummarySourceConfig) => void;
  onTimeChange: (hour: number, minute: number) => void;
  onReporterModeChange: (mode: SummaryReporterMode) => void;
  onReporterChange: (pubkey: string | null) => void;

  onSave: () => void;
  onCancel?: () => void;
};

const NECTAR_SELECT_VALUE = "nectar";

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
  onRuntimeDraftChange,
  runtimeValid,
  saving,
  error,
  onSourceConfigChange,
  onTimeChange,
  onReporterModeChange,
  onReporterChange,

  onSave,
  onCancel,
}: SummaryBuilderProps) {
  const { channels, dms } = partitionLiveConversations(conversations);
  const timeValue = formatLocalTime(hour, minute);
  const [step, setStep] = React.useState(0);
  const [editAgentOpen, setEditAgentOpen] = React.useState(false);
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const [agentDraft, setAgentDraft] = React.useState<string[]>([]);
  const selectedAgent =
    reporterMode === "nectar"
      ? agents.find((agent) => agent.personaId === "nectar")
      : agents.find((agent) => agent.pubkey === reporterPubkey);
  const showNectarSetup = reporterMode === "nectar" && !nectarExists;

  return (
    <fieldset disabled={saving} className="flex min-w-0 flex-col gap-8">
      <p className="text-sm text-muted-foreground">
        Step {step + 1} of 3 ·{" "}
        {["Agent", "Conversations", "Schedule and review"][step]}
      </p>
      <div>
        <h2 className="text-lg font-semibold tracking-tight">
          Build your Summary
        </h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Choose the conversations you belong to. New rooms you join can be
          preselected when you edit this setup. Archived and deleted rooms drop
          off on their own.
        </p>
      </div>

      <div hidden={step !== 0} className="space-y-4">
        <SettingsOptionGroup
          description="Use Nectar or choose one of your agents to write the daily Summary."
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
          <SettingsOptionRow>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium">Agent</p>
              <p className="text-sm text-muted-foreground">
                {reporterMode === "nectar"
                  ? NECTAR_DISPLAY_NAME
                  : (selectedAgent?.name ?? "Choose an agent")}
              </p>
            </div>
            <Button
              aria-haspopup="dialog"
              onClick={() => {
                setAgentDraft([
                  reporterMode === "nectar"
                    ? NECTAR_SELECT_VALUE
                    : (reporterPubkey ?? ""),
                ]);
                setPickerOpen(true);
              }}
              variant="outline"
              disabled={saving}
            >
              <UsersRound aria-hidden="true" className="size-4" />
              Choose agent
            </Button>
          </SettingsOptionRow>
          <ManagedAgentPickerDialog
            open={pickerOpen}
            onOpenChange={setPickerOpen}
            title="Summary agent"
            description="Choose the agent that writes your Summary."
            selected={agentDraft}
            onSelectionChange={setAgentDraft}
            multiple={false}
            testId="summary-agent-picker"
            saving={saving}
            extraAgents={[
              {
                pubkey: NECTAR_SELECT_VALUE,
                name: "Nectar (Summary agent)",
                avatarUrl: NECTAR_AVATAR_PATH,
              },
            ]}
            onConfirm={() => {
              const key = agentDraft[0];
              onReporterModeChange(
                key === NECTAR_SELECT_VALUE ? "nectar" : "existing",
              );
              onReporterChange(key === NECTAR_SELECT_VALUE ? null : key);
              setPickerOpen(false);
            }}
          />
          {selectedAgent ? (
            <>
              <Button
                disabled={saving}
                variant="outline"
                onClick={() => setEditAgentOpen(true)}
              >
                Edit agent setup
              </Button>
              {editAgentOpen ? (
                <AgentDialog
                  mode="instance-edit"
                  agent={selectedAgent}
                  open={editAgentOpen}
                  onOpenChange={setEditAgentOpen}
                />
              ) : null}
            </>
          ) : null}
          {showNectarSetup ? (
            <SummaryRuntimeFields
              disabled={saving}
              onChange={onRuntimeDraftChange}
            />
          ) : null}
        </SettingsOptionGroup>
      </div>
      <div hidden={step !== 1} className="space-y-6">
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
          description="Allow the Summary agent to read these conversations. This does not add it as a participant or allow it to send messages."
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
      </div>
      <div hidden={step !== 2}>
        <p className="mb-4 text-sm">
          {reporterMode === "nectar" ? "Nectar" : selectedAgent?.name} will
          summarize{" "}
          {
            conversations.filter((c) => isConversationChecked(sourceConfig, c))
              .length
          }{" "}
          conversations daily. Selected DMs are shared with this agent for
          read-only Summary access.
        </p>
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
              <p
                className="text-sm font-medium"
                id="summary-auto-include-label"
              >
                Preselect new conversations
              </p>
              <p
                className="text-xs text-muted-foreground/70"
                data-settings-subcopy
              >
                New conversations are preselected when you return to setup. Save
                to update the agent’s access.
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
      </div>
      {error ? (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}

      <div className="flex items-center gap-2">
        {step > 0 ? (
          <Button
            variant="outline"
            disabled={saving}
            onClick={() => setStep(step - 1)}
          >
            Back
          </Button>
        ) : null}
        <Button
          disabled={
            saving ||
            (showNectarSetup && !runtimeValid) ||
            (reporterMode === "existing" && !reporterPubkey)
          }
          onClick={step < 2 ? () => setStep(step + 1) : onSave}
          type="button"
        >
          {saving ? "Saving…" : step < 2 ? "Continue" : "Save Summary"}
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
    </fieldset>
  );
}
