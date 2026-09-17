import * as React from "react";
import {
  ArrowLeft,
  ArrowRight,
  BellRing,
  Bot,
  MessageSquarePlus,
} from "lucide-react";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/shared/ui/dialog";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import {
  AUTOMATION_TASKS,
  automationTasks,
  initialAutomation,
  type AutomationTask,
} from "../automationPresentation";
import { activeRecords, type AutomationControls } from "../recordTypes";
import { publishPlanningRecord } from "../recordMutations";
import type { DispatchContext } from "../types";
import { AutomationSummary } from "./AutomationSummary";
import { AutomationWorkPicker } from "./AutomationWorkPicker";
import { DispatchCapacityEditor } from "./DispatchCapacityEditor";

export const AUTOMATION_ICONS = {
  capture: MessageSquarePlus,
  remind: BellRing,
  assign: Bot,
};
export const automationSelectClass =
  "h-10 w-full rounded-md border border-input bg-background px-3 text-sm";

export function AutomationSetup({
  context,
  workspace,
  initial,
  preset,
  onClose,
  refresh,
  onSaved,
}: {
  context: DispatchContext;
  workspace: string;
  initial?: AutomationControls;
  preset?: AutomationTask;
  onClose: () => void;
  refresh: () => Promise<DispatchContext>;
  onSaved: () => void;
}) {
  const { channels } = useChannelNavigation();
  const connections = activeRecords(context.records, "automation");
  const preferred =
    initial ??
    (connections.length === 1
      ? connections[0].record.payload.controls
      : undefined);
  const [draft, setDraft] = React.useState(() =>
    initialAutomation(workspace, preferred),
  );
  const [selected, setSelected] = React.useState<AutomationTask[]>(
    preset ? [preset] : initial ? automationTasks(initial) : [],
  );
  const [step, setStep] = React.useState(0),
    [busy, setBusy] = React.useState(false),
    [agentSettingsOpen, setAgentSettingsOpen] = React.useState(false),
    [error, setError] = React.useState<string | null>(null);
  const heading = React.useCallback((node: HTMLHeadingElement | null) => {
    node?.focus();
  }, []);
  const agentEscapeAction = React.useRef<(() => void) | null>(null);
  const setAgentEscapeAction = React.useCallback(
    (action: (() => void) | null) => {
      agentEscapeAction.current = action;
    },
    [],
  );
  const profiles = useUsersBatchQuery(
    connections.map((value) => value.record.payload.controls.service_pubkey),
  );
  const connectionName = (key: string, index: number) =>
    resolveUserLabel({
      pubkey: key,
      profiles: profiles.data?.profiles,
      fallbackName: `Workspace connection ${index + 1}`,
    });
  const update = <K extends keyof AutomationControls>(
    key: K,
    value: AutomationControls[K],
  ) => setDraft((current) => ({ ...current, [key]: value }));
  const choose = (task: AutomationTask, checked: boolean) => {
    setSelected((current) =>
      checked ? [...current, task] : current.filter((id) => id !== task),
    );
    if (
      checked &&
      task === "remind" &&
      !draft.notify_triage &&
      draft.unassigned_after_secs === null
    )
      update("unassigned_after_secs", 86400);
    if (checked && task === "assign" && !draft.routing.length)
      update("routing", [{ label: "", executor: "", channel: workspace }]);
  };
  // Presets select a task, but never inherit hidden actions from a previous setup.
  React.useEffect(() => {
    if (preset === "remind")
      setDraft((current) => ({
        ...current,
        unassigned_after_secs: current.unassigned_after_secs ?? 86400,
      }));
    if (preset === "assign")
      setDraft((current) => ({
        ...current,
        routing: current.routing.length
          ? current.routing
          : [{ label: "", executor: "", channel: workspace }],
      }));
  }, [preset, workspace]);
  const controls: AutomationControls = {
    ...draft,
    mode: "preview",
    expires_at: null,
    capture_channels: selected.includes("capture")
      ? draft.capture_channels
      : [],
    issue_ids:
      selected.includes("assign") || selected.includes("remind")
        ? draft.issue_ids
        : [],
    routing: selected.includes("assign") ? draft.routing : [],
    notify_triage: selected.includes("remind") && draft.notify_triage,
    unassigned_after_secs: selected.includes("remind")
      ? draft.unassigned_after_secs
      : null,
  };
  const needsIssues =
    selected.includes("assign") || selected.includes("remind");
  const validLimit =
    Number.isInteger(draft.max_actions_total) &&
    draft.max_actions_total >= 1 &&
    draft.max_actions_total <= 50;
  const connectionValid =
    /^[a-z0-9_-]{1,64}$/.test(draft.service_id) &&
    /^[0-9a-f]{64}$/.test(draft.service_pubkey);
  const problem = !connectionValid
    ? "An administrator needs to connect automation before you can continue."
    : selected.includes("capture") && !controls.capture_channels.length
      ? "Choose at least one channel for messages."
      : needsIssues && !controls.issue_ids.length
        ? "Choose the issues Buzz should follow up on."
        : selected.includes("assign") &&
            (!controls.routing.length ||
              controls.routing.some(
                (route) => !route.label || !route.executor || !route.channel,
              ))
          ? "Choose a label, agent, and channel for each assignment rule."
          : selected.includes("remind") &&
              !controls.notify_triage &&
              controls.unassigned_after_secs === null
            ? "Choose when Buzz should send a reminder."
            : controls.unassigned_after_secs !== null &&
                (!Number.isInteger(controls.unassigned_after_secs) ||
                  controls.unassigned_after_secs < 3600 ||
                  controls.unassigned_after_secs > 30 * 86400)
              ? "Choose a wait between 1 and 720 hours."
              : !validLimit
                ? "Choose a limit between 1 and 50 actions."
                : null;
  const unavailableChoice = controls.capture_channels.some(
    (id) => !channels.some((channel) => channel.id === id),
  )
    ? "Remove the unavailable channel or choose one you can access."
    : controls.issue_ids.some(
          (id) => !context.issues.some((issue) => issue.issueId === id),
        )
      ? "Remove the unavailable issue or choose current work."
      : controls.routing.some(
            (route) =>
              !channels.some((channel) => channel.id === route.channel),
          ) ||
          (selected.includes("remind") &&
            !channels.some(
              (channel) => channel.id === controls.notification_channel,
            ))
        ? "Choose an available channel for the reminder or assignment."
        : new Set(controls.routing.map((route) => route.label)).size !==
            controls.routing.length
          ? "Use each issue label once so Buzz knows which agent should take it."
          : null;
  const validationMessage = problem ?? unavailableChoice;
  const replacing = connections.find(
    (value) =>
      value.record.payload.controls.service_id === draft.service_id &&
      value.record.payload.controls.service_pubkey === draft.service_pubkey,
  );
  const save = async () => {
    if (validationMessage || !selected.length) return;
    setBusy(true);
    setError(null);
    try {
      await publishPlanningRecord({
        workspace,
        payload: { type: "automation", controls },
        refresh,
      });
      onSaved();
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy && !agentSettingsOpen) onClose();
      }}
    >
      <DialogContent
        className="flex max-h-[90vh] max-w-2xl flex-col overflow-hidden p-0"
        data-testid="automation-setup"
        showCloseButton={!agentSettingsOpen}
        onEscapeKeyDown={(event) => {
          // The nested picker or editor owns Escape until its work is dismissed.
          if (!event.defaultPrevented && agentEscapeAction.current) {
            event.preventDefault();
            agentEscapeAction.current();
          }
        }}
      >
        <DialogHeader className="px-6 pt-6 pr-12">
          <DialogTitle>Set up automation</DialogTitle>
          <DialogDescription>
            Build a setup and review what it will do before turning it on.
          </DialogDescription>
        </DialogHeader>
        <ol aria-label="Setup progress" className="flex gap-2 px-6 text-xs">
          {["Choose a job", "Choose the details", "Review"].map(
            (title, index) => (
              <li
                key={title}
                aria-current={index === step ? "step" : undefined}
                className={`flex-1 rounded-md px-2 py-2 ${index === step ? "bg-primary/10 font-semibold text-primary" : "bg-muted/40 text-muted-foreground"}`}
              >
                {index + 1}. {title}
              </li>
            ),
          )}
        </ol>
        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-2">
          <h3
            key={step}
            ref={heading}
            tabIndex={-1}
            className="text-lg font-semibold outline-none"
          >
            {
              [
                "What would you like Buzz to do?",
                "Where should Buzz help?",
                "Does this look right?",
              ][step]
            }
          </h3>
          <fieldset disabled={busy} className="min-w-0 space-y-5">
            {step === 0 ? (
              <div className="space-y-3">
                <p className="text-sm text-muted-foreground">
                  Choose one job to start, or combine jobs that belong together.
                </p>
                {AUTOMATION_TASKS.map((task) => {
                  const Icon = AUTOMATION_ICONS[task.id];
                  return (
                    <label
                      key={task.id}
                      className={`flex cursor-pointer items-start gap-3 rounded-xl border p-4 ${selected.includes(task.id) ? "border-primary bg-primary/5" : "border-border hover:bg-muted/30"}`}
                    >
                      <input
                        type="checkbox"
                        className="mt-1 size-4 shrink-0 accent-primary"
                        aria-label={task.title}
                        checked={selected.includes(task.id)}
                        onChange={(event) =>
                          choose(task.id, event.target.checked)
                        }
                      />
                      <Icon
                        className="mt-0.5 size-5 shrink-0 text-muted-foreground"
                        aria-hidden="true"
                      />
                      <span>
                        <span className="block text-sm font-semibold">
                          {task.title}
                        </span>
                        <span className="mt-1 block text-sm text-muted-foreground">
                          {task.description}
                        </span>
                        <span className="mt-2 block text-xs text-muted-foreground">
                          For example: {task.example}
                        </span>
                      </span>
                    </label>
                  );
                })}
              </div>
            ) : null}
            {step === 1 ? (
              <>
                {!connectionValid ? (
                  <div className="rounded-lg border border-border bg-muted/40 p-4 text-sm">
                    <p className="font-medium">
                      Automation needs to be connected
                    </p>
                    <p className="mt-1 text-muted-foreground">
                      Ask the person who manages your Buzz workspace to connect
                      automation. You can explore the options below while they
                      set it up.
                    </p>
                  </div>
                ) : null}
                {connections.length > 1 ? (
                  <label className="grid gap-1 text-sm">
                    Workspace connection
                    <select
                      className={automationSelectClass}
                      value={
                        connections.find(
                          (value) =>
                            value.record.payload.controls.service_id ===
                              draft.service_id &&
                            value.record.payload.controls.service_pubkey ===
                              draft.service_pubkey,
                        )?.event_id ?? ""
                      }
                      onChange={(event) => {
                        const connection = connections.find(
                          (value) => value.event_id === event.target.value,
                        )?.record.payload.controls;
                        if (connection)
                          setDraft((current) => ({
                            ...current,
                            service_id: connection.service_id,
                            service_pubkey: connection.service_pubkey,
                          }));
                      }}
                    >
                      <option value="">Choose a connection</option>
                      {connections.map((value, index) => (
                        <option key={value.event_id} value={value.event_id}>
                          {connectionName(
                            value.record.payload.controls.service_pubkey,
                            index,
                          )}
                        </option>
                      ))}
                    </select>
                  </label>
                ) : null}
                <AutomationWorkPicker
                  context={context}
                  controls={draft}
                  selected={selected}
                  workspace={workspace}
                  update={update}
                />
                {selected.includes("assign") ? (
                  <DispatchCapacityEditor
                    inline
                    onEditingChange={setAgentSettingsOpen}
                    onEscapeActionChange={setAgentEscapeAction}
                    context={context}
                    workspace={workspace}
                    refresh={refresh}
                  />
                ) : null}
                <div className="rounded-xl border border-border p-4">
                  <h4 className="text-sm font-semibold">
                    Start with a small run
                  </h4>
                  <p className="mt-1 text-sm text-muted-foreground">
                    Buzz will stop after one hour or when this limit is reached.
                    You can review the results before doing more.
                  </p>
                  <label
                    className="mt-3 grid max-w-48 gap-1 text-sm"
                    htmlFor="automation-action-limit"
                  >
                    Stop after this many actions
                    <Input
                      id="automation-action-limit"
                      type="number"
                      min={1}
                      max={50}
                      value={draft.max_actions_total}
                      onChange={(event) =>
                        setDraft((current) => ({
                          ...current,
                          max_actions_total: Number(event.target.value),
                          max_actions_per_tick: Math.min(
                            current.max_actions_per_tick,
                            Math.max(1, Number(event.target.value)),
                          ),
                        }))
                      }
                    />
                  </label>
                  <p className="mt-2 text-xs text-muted-foreground">
                    Each saved issue, assignment, or reminder counts as one
                    action.
                  </p>
                </div>
                <details className="text-xs text-muted-foreground">
                  <summary className="cursor-pointer">
                    Connection settings for administrators
                  </summary>
                  <p className="my-2">
                    Use the name and public key of the dispatcher already
                    configured for this workspace. These fields connect to an
                    existing service; they do not install one. Its allowed
                    channels and agents must include the choices above.
                  </p>
                  <label
                    className="mt-2 grid gap-1"
                    htmlFor="automation-service-name"
                  >
                    Service name
                    <Input
                      id="automation-service-name"
                      value={draft.service_id}
                      onChange={(event) =>
                        update("service_id", event.target.value.trim())
                      }
                      maxLength={64}
                    />
                  </label>
                  <label
                    className="mt-2 grid gap-1"
                    htmlFor="automation-service-key"
                  >
                    Service public key
                    <Input
                      id="automation-service-key"
                      value={draft.service_pubkey}
                      onChange={(event) =>
                        update("service_pubkey", event.target.value.trim())
                      }
                      maxLength={64}
                    />
                  </label>
                  <p className="mt-2">
                    Ask your Buzz administrator if you do not have these
                    connection details.
                  </p>
                </details>
              </>
            ) : null}
            {step === 2 ? (
              <>
                <div className="rounded-xl border border-border bg-muted/20 p-4">
                  <AutomationSummary controls={controls} />
                </div>
                {needsIssues ? (
                  <div className="text-sm">
                    <h4 className="font-medium">
                      {controls.issue_ids.length === 1
                        ? "Only this selected issue"
                        : `Only these ${controls.issue_ids.length} selected issues`}
                    </h4>
                    <ul className="mt-2 max-h-36 list-inside list-disc space-y-1 overflow-y-auto text-muted-foreground">
                      {controls.issue_ids.map((id) => (
                        <li key={id}>
                          {context.issues.find((issue) => issue.issueId === id)
                            ?.title ?? "Unavailable issue"}
                        </li>
                      ))}
                    </ul>
                  </div>
                ) : null}
                <p className="rounded-lg bg-muted/40 p-3 text-sm">
                  Stop after up to{" "}
                  <strong>{controls.max_actions_total} actions</strong> or{" "}
                  <strong>one hour</strong>, whichever comes first.
                </p>
                {controls.capture_channels.length ? (
                  <p className="text-sm text-muted-foreground">
                    Older 📌 reactions are left alone. Reactions added after you
                    save this setup are included when you turn it on. Messages
                    are copied only if everyone in this Dispatch workspace can
                    see the source channel.
                  </p>
                ) : null}
                {controls.routing.length ? (
                  <p className="text-sm text-muted-foreground">
                    Buzz sends work only when it is in Todo, its blockers are
                    resolved, and the agent has room. The issue owner reviews
                    the result.
                  </p>
                ) : null}
                {replacing ? (
                  <p className="rounded-lg border border-border p-3 text-sm">
                    <strong>
                      This replaces the previous setup for this connection.
                    </strong>{" "}
                    The new setup stays off until you turn it on. Work already
                    sent to agents may continue.
                  </p>
                ) : null}
                <p className="text-sm font-medium">
                  Saving is safe to try: Buzz will check the setup, then you can
                  choose “Turn on”.
                </p>
              </>
            ) : null}
          </fieldset>
          {error ? (
            <div
              role="alert"
              className="rounded-lg border border-destructive/40 p-3 text-sm"
            >
              <p>
                Buzz could not confirm this save. Your choices are still here.
                Try saving again; Buzz will reuse the saved change.
              </p>
              <details className="mt-2 text-xs">
                <summary>Show details</summary>
                {error}
              </details>
            </div>
          ) : null}
        </div>
        <div className="border-t border-border px-6 py-4">
          {agentSettingsOpen ? (
            <p className="mb-3 text-sm text-muted-foreground" role="status">
              Save or cancel the agent settings above to continue.
            </p>
          ) : step === 1 && validationMessage ? (
            <p className="mb-3 text-sm text-muted-foreground" role="status">
              {validationMessage}
            </p>
          ) : null}
          <div className="flex flex-col-reverse gap-2 sm:flex-row sm:items-center sm:justify-between sm:gap-3">
            <Button
              variant="ghost"
              disabled={busy || agentSettingsOpen}
              onClick={() => (step === 0 ? onClose() : setStep(step - 1))}
            >
              {step > 0 ? (
                <ArrowLeft className="size-4" aria-hidden="true" />
              ) : null}
              {step === 0 ? "Cancel" : "Back"}
            </Button>
            {step < 2 ? (
              <Button
                disabled={
                  busy ||
                  agentSettingsOpen ||
                  !selected.length ||
                  (step === 1 && Boolean(validationMessage))
                }
                onClick={() => setStep(step + 1)}
              >
                {step === 0 ? "Choose the details" : "Review setup"}
                <ArrowRight className="size-4" aria-hidden="true" />
              </Button>
            ) : (
              <Button
                disabled={busy || Boolean(validationMessage)}
                onClick={() => void save()}
              >
                {busy ? "Saving…" : "Save and check setup"}
              </Button>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
