import { SummaryScheduleFields } from "./SummaryScheduleFields";
import {
  summaryScheduleError,
  type SummarySchedule,
} from "../lib/summarySchedule";
import { AgentDialog } from "@/features/agents/ui/AgentDialog";
import * as React from "react";
import { SlidersHorizontal, UsersRound } from "lucide-react";
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
  isConversationChecked,
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
  identityFields?: React.ReactNode;
  audienceFields?: React.ReactNode;
  reviewFields?: React.ReactNode;
  detailsValid?: boolean;
  conversations: SummaryConversation[];
  sourceConfig: SummarySourceConfig;
  schedule: SummarySchedule | null;
  timezone: string;
  onTimezoneChange?: (timezone: string) => void;
  reporterMode: SummaryReporterMode;
  reporterPubkey: string | null;
  agents: ManagedAgent[];
  nectarExists: boolean;
  onRuntimeDraftChange: (draft: SummaryRuntimeDraft | null) => void;
  runtimeValid: boolean;
  saving: boolean;
  error: string | null;
  onSourceConfigChange: (config: SummarySourceConfig) => void;
  onScheduleChange: (schedule: SummarySchedule | null) => void;
  onReporterModeChange: (mode: SummaryReporterMode) => void;
  onReporterChange: (pubkey: string | null) => void;

  onValidateSources?: () => Promise<unknown>;
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
  identityFields,
  audienceFields,
  reviewFields,
  detailsValid = true,
  sourceConfig,
  schedule,
  timezone,
  onTimezoneChange,
  reporterMode,
  reporterPubkey,
  agents,
  nectarExists,
  onRuntimeDraftChange,
  runtimeValid,
  saving,
  error,
  onSourceConfigChange,
  onScheduleChange,
  onReporterModeChange,
  onReporterChange,

  onSave,
  onValidateSources,
  onCancel,
}: SummaryBuilderProps) {
  const [search, setSearch] = React.useState("");
  const [selectedOnly, setSelectedOnly] = React.useState(false);
  const visible = conversations.filter(
    (c) =>
      c.name.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase()) &&
      (!selectedOnly || isConversationChecked(sourceConfig, c)),
  );
  const { channels, dms } = partitionLiveConversations(visible);
  const timezoneValid = (() => {
    try {
      new Intl.DateTimeFormat(undefined, { timeZone: timezone });
      return true;
    } catch {
      return false;
    }
  })();
  const selectedCount = conversations.filter((c) =>
    isConversationChecked(sourceConfig, c),
  ).length;
  const setVisible = (checked: boolean) =>
    onSourceConfigChange(
      [...channels, ...dms].reduce(
        (config, c) => toggleConversation(config, c, checked),
        sourceConfig,
      ),
    );
  const [step, setStep] = React.useState(0);
  const [checking, setChecking] = React.useState(false);
  const [validationError, setValidationError] = React.useState<string | null>(
    null,
  );
  const [editAgentOpen, setEditAgentOpen] = React.useState(false);
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const [agentDraft, setAgentDraft] = React.useState<string[]>([]);
  const selectedAgent =
    reporterMode === "nectar"
      ? agents.find((agent) => agent.personaId === "nectar")
      : agents.find((agent) => agent.pubkey === reporterPubkey);
  const showNectarSetup = reporterMode === "nectar" && !nectarExists;

  return (
    <fieldset
      disabled={saving || checking}
      className="flex min-w-0 flex-col gap-8"
    >
      <p className="text-sm text-muted-foreground">
        Step {step + 1} of {audienceFields ? 4 : 3} ·{" "}
        {
          (audienceFields
            ? [
                "Name and agent",
                "Audience",
                "Conversations",
                "Schedule and review",
              ]
            : ["Agent", "Conversations", "Schedule and review"])[step]
        }
      </p>
      <div>
        <h2 className="text-lg font-semibold tracking-tight">
          Build your Summary
        </h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Choose the conversations you belong to. New rooms you join can be
          preselected when you edit this setup. If a selected room becomes
          unavailable, review and save your conversations to update the
          reporting scope.
        </p>
      </div>

      <div hidden={step !== 0} className="space-y-4">
        {identityFields}
        <SettingsOptionGroup
          description="Use Nectar or choose one of your agents to write your Summary."
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
          <SettingsOptionRow className="flex-wrap gap-3">
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium">Agent</p>
              <p className="text-sm text-muted-foreground">
                {reporterMode === "nectar"
                  ? NECTAR_DISPLAY_NAME
                  : (selectedAgent?.name ?? "Choose an agent")}
              </p>
            </div>
            <div className="flex shrink-0 flex-wrap items-center gap-2">
              {selectedAgent ? (
                <Button
                  disabled={saving}
                  variant="ghost"
                  size="sm"
                  onClick={() => setEditAgentOpen(true)}
                >
                  <SlidersHorizontal aria-hidden="true" className="size-4" />
                  Edit agent setup
                </Button>
              ) : null}
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
            </div>
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
            <p className="px-4 pb-3 text-sm text-muted-foreground">
              Editing this agent saves immediately and affects every
              conversation and Summary that uses it. Cancelling this Summary
              setup will not undo agent changes.
            </p>
          ) : null}
          {selectedAgent && editAgentOpen ? (
            <AgentDialog
              mode="instance-edit"
              agent={selectedAgent}
              open={editAgentOpen}
              onOpenChange={setEditAgentOpen}
            />
          ) : null}
          {showNectarSetup ? (
            <SummaryRuntimeFields
              disabled={saving}
              onChange={onRuntimeDraftChange}
            />
          ) : null}
        </SettingsOptionGroup>
      </div>
      {audienceFields ? <div hidden={step !== 1}>{audienceFields}</div> : null}
      <div hidden={step !== (audienceFields ? 2 : 1)} className="space-y-6">
        <div className="space-y-3">
          <Input
            aria-label="Search conversations"
            placeholder="Search channels and direct messages"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          <div className="flex flex-wrap items-center gap-3">
            <span className="text-sm">
              {selectedCount} of 256 conversations selected
            </span>
            <Button
              variant="outline"
              onClick={() => setSelectedOnly(!selectedOnly)}
              aria-pressed={selectedOnly}
            >
              Selected only
            </Button>
            <Button variant="ghost" onClick={() => setVisible(true)}>
              Select visible
            </Button>
            <Button variant="ghost" onClick={() => setVisible(false)}>
              Clear visible
            </Button>
          </div>
        </div>
        <SettingsOptionGroup title="Channels">
          {channels.length === 0 ? (
            <SettingsOptionRow>
              <p className="text-sm text-muted-foreground">
                No channels match your filters.
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
                No direct messages match your filters.
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
      <div hidden={step !== (audienceFields ? 3 : 2)}>
        {reviewFields}
        <details className="mb-4 rounded-lg border p-3">
          <summary className="cursor-pointer text-sm font-medium">
            Review {selectedCount} selected conversations
          </summary>
          <ul className="mt-2 max-h-48 overflow-auto text-sm">
            {conversations
              .filter((c) => isConversationChecked(sourceConfig, c))
              .map((c) => (
                <li key={c.id}>
                  {c.kind === "channel" ? "#" : ""}
                  {c.name}
                </li>
              ))}
          </ul>
        </details>
        <p className="mb-4 text-sm">
          {reporterMode === "nectar" ? "Nectar" : selectedAgent?.name} will
          summarize{" "}
          {
            conversations.filter((c) => isConversationChecked(sourceConfig, c))
              .length
          }{" "}
          conversations. Selected DMs are shared with this agent for read-only
          Summary access.
        </p>
        <p className="mb-4 text-sm text-muted-foreground">
          The first report covers the previous 24 hours. Later reports cover
          unread messages since the last successful report for each
          conversation.
        </p>
        {selectedCount === 0 ? (
          <p role="alert" className="mb-4 text-sm text-destructive">
            Select at least one conversation before saving a scheduled Summary.
          </p>
        ) : null}
        <SettingsOptionGroup
          description="Choose when to run automatically, or use manual runs only."
          title="Schedule"
        >
          <div className="p-4">
            <SummaryScheduleFields
              schedule={schedule}
              timezone={timezone}
              onTimezoneChange={onTimezoneChange}
              disabled={saving}
              onChange={onScheduleChange}
            />
          </div>
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
      {validationError ? (
        <p role="alert" className="text-sm text-destructive">
          {validationError}
        </p>
      ) : null}
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
            !detailsValid ||
            (step === (audienceFields ? 3 : 2) &&
              (!timezoneValid ||
                Boolean(summaryScheduleError(schedule)) ||
                (schedule !== null && selectedCount === 0))) ||
            (showNectarSetup && !runtimeValid) ||
            (reporterMode === "existing" && !reporterPubkey)
          }
          onClick={
            step < (audienceFields ? 3 : 2)
              ? async () => {
                  setValidationError(null);
                  if (step === (audienceFields ? 2 : 1) && onValidateSources) {
                    setChecking(true);
                    try {
                      await onValidateSources();
                      setStep(step + 1);
                    } catch (error) {
                      setValidationError(
                        error instanceof Error
                          ? error.message
                          : "Could not validate conversations.",
                      );
                    } finally {
                      setChecking(false);
                    }
                  } else setStep(step + 1);
                }
              : onSave
          }
          type="button"
        >
          {checking
            ? "Checking access…"
            : saving
              ? "Saving…"
              : step < (audienceFields ? 3 : 2)
                ? "Continue"
                : "Save Summary"}
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
