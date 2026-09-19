import { RelaySettingRecovery } from "./RelaySettingRecovery";
import * as React from "react";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import { activeRecords, type AutopilotAuthorization } from "../recordTypes";
import { publishPlanningRecord } from "../recordMutations";
import {
  autopilotReviewLine,
  saveRelayAuthorization,
} from "../relayAutomation";
import type { DispatchContext } from "../types";
import { AssigneePickerDialog } from "./AssigneePickerDialog";

export function AutopilotCard({
  context,
  workspace,
  project,
  refresh,
  onOpenIssue,
}: {
  context: DispatchContext;
  workspace: string;
  project: string;
  refresh: () => Promise<DispatchContext>;
  onOpenIssue: (id: string) => void;
}) {
  const matches = activeRecords(context.records, "autopilot").filter(
    (v) => v.record.payload.project === project,
  );
  const current = matches[0];
  const [draft, setDraft] = React.useState<AutopilotAuthorization>(
    () =>
      current?.record.payload ?? {
        type: "autopilot",
        workspace,
        project,
        agents: [],
        label: null,
        at_a_time: 2,
        per_day: 10,
        reviewer_required: true,
        enabled: false,
        activation_id: crypto.randomUUID(),
        confirmation_required: true,
        confirmation_event: null,
      },
  );
  const [base, setBase] = React.useState(current);
  const [dirty, setDirty] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [picker, setPicker] = React.useState(false);
  const [capacity, setCapacity] = React.useState(2);
  const [error, setError] = React.useState<string | null>(null);
  const retryIntent = React.useRef({ enabled: false, confirm: false });
  const activationIntent = React.useRef<string | null>(null);
  const profiles = useUsersBatchQuery(draft.agents);
  const label = (key: string) =>
    resolveUserLabel({
      pubkey: key,
      profiles: profiles.data?.profiles,
      fallbackName: "Selected agent",
    });
  const capacities = activeRecords(context.records, "capacity");
  React.useEffect(() => {
    if (!dirty && current) {
      setDraft(current.record.payload);
      setBase(current);
    }
  }, [current, dirty]);
  const change = (patch: Partial<AutopilotAuthorization>) => {
    setDraft((value) => ({ ...value, ...patch }));
    setDirty(true);
  };
  const editable =
    context.canAuthorizeAutomation === true &&
    !context.sectionErrors.records &&
    matches.length <= 1;
  const challenge = context.automation?.outcomes?.find(
    (value) =>
      value.requestKey ===
        `confirmation:${current?.record.payload.activation_id}` &&
      value.confirmationRequired &&
      value.confirmationEvent,
  );
  const needsConfirmation =
    current?.record.payload.enabled &&
    current.record.payload.confirmation_required &&
    challenge;
  const actions = (context.automation?.actions ?? [])
    .filter(
      (value) =>
        value.signer === context.automation?.relaySigner &&
        ["assign", "escalate"].includes(value.action) &&
        context.issues.some(
          (issue) =>
            issue.issueId === value.issueId && issue.projectId === project,
        ),
    )
    .sort(
      (a, b) => b.createdAt - a.createdAt || b.eventId.localeCompare(a.eventId),
    );
  async function save(enabled: boolean, confirm = false) {
    if (busy) return;
    retryIntent.current = { enabled, confirm };
    setBusy(true);
    setError(null);
    try {
      if (
        enabled &&
        (!draft.agents.length ||
          draft.agents.length > 32 ||
          !Number.isInteger(draft.at_a_time) ||
          draft.at_a_time < 1 ||
          draft.at_a_time > 32 ||
          !Number.isInteger(draft.per_day) ||
          draft.per_day < 1 ||
          draft.per_day > 1000 ||
          draft.label === "")
      )
        throw new Error(
          "Choose an agent, a label if filtering, and valid limits before turning on Autopilot.",
        );
      if (!Number.isInteger(capacity) || capacity < 1 || capacity > 32)
        throw new Error("Choose agent capacity between 1 and 32 issues.");
      let fresh = await refresh();
      if (enabled)
        for (const executor of draft.agents) {
          if (
            !activeRecords(fresh.records, "capacity").some(
              (v) => v.record.payload.executor === executor,
            )
          ) {
            await publishPlanningRecord({
              workspace,
              payload: {
                type: "capacity",
                executor,
                max_active: capacity,
                capabilities: [],
                paused: false,
              },
              refresh,
            });
            fresh = await refresh();
          }
        }
      const previous = confirm ? current : base;
      const payload: AutopilotAuthorization = { ...draft, enabled };
      if (enabled && !previous?.record.payload.enabled) {
        activationIntent.current ??= crypto.randomUUID();
        payload.activation_id = activationIntent.current;
        payload.confirmation_required = true;
        payload.confirmation_event = null;
      }
      if (confirm) {
        if (!challenge?.confirmationEvent || !current)
          throw new Error(
            "Refresh to load the assignment awaiting confirmation.",
          );
        Object.assign(payload, current.record.payload, {
          enabled: true,
          confirmation_required: false,
          confirmation_event: challenge.confirmationEvent,
        });
      }
      const accepted = await saveRelayAuthorization({
        payload,
        previous,
        refresh,
      });
      if (accepted?.record.payload.type === "autopilot") {
        setDraft(accepted.record.payload);
        setBase(accepted as typeof current);
      }
      setDirty(false);
      activationIntent.current = null;
      await refresh();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section
      className="mt-5 space-y-4 rounded-xl border border-border p-4"
      data-testid="dispatch-autopilot"
    >
      <div>
        <h3 className="font-semibold">Autopilot</h3>
        <p className="mt-1 text-sm text-muted-foreground">
          Let an agent work through this project's ready issues.
        </p>
      </div>
      <fieldset disabled={!editable || busy} className="space-y-4">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm">Agent(s)</span>
          {draft.agents.map((key) => (
            <Button
              key={key}
              size="sm"
              variant="outline"
              onClick={() =>
                change({ agents: draft.agents.filter((v) => v !== key) })
              }
              aria-label={`Remove ${label(key)}`}
            >
              {label(key)} ×
            </Button>
          ))}
          <Button size="sm" variant="outline" onClick={() => setPicker(true)}>
            Choose an agent
          </Button>
        </div>
        {draft.agents.some(
          (key) => !capacities.some((v) => v.record.payload.executor === key),
        ) ? (
          <label
            htmlFor="autopilot-capacity"
            className="grid max-w-xs gap-1 text-sm"
          >
            Issues at the same time
            <Input
              id="autopilot-capacity"
              type="number"
              min={1}
              max={32}
              value={capacity}
              onChange={(e) => setCapacity(Number(e.target.value))}
            />
          </label>
        ) : null}
        <fieldset className="flex flex-wrap gap-3 text-sm">
          <legend className="mb-2">Which work</legend>
          <label>
            <input
              type="radio"
              name={`work-${project}`}
              checked={draft.label === null}
              onChange={() => change({ label: null })}
            />{" "}
            All ready issues
          </label>
          <label>
            <input
              type="radio"
              name={`work-${project}`}
              checked={draft.label !== null}
              onChange={() => change({ label: "" })}
            />{" "}
            Only issues labeled
          </label>
          {draft.label !== null ? (
            <Input
              className="max-w-xs"
              aria-label="Issue label"
              maxLength={64}
              value={draft.label}
              onChange={(e) =>
                change({ label: e.target.value.trim().toLowerCase() })
              }
            />
          ) : null}
        </fieldset>
        <div className="flex flex-wrap items-end gap-4">
          <label
            htmlFor="autopilot-concurrent"
            className="grid w-28 gap-1 text-sm"
          >
            At a time
            <Input
              id="autopilot-concurrent"
              type="number"
              min={1}
              max={32}
              value={draft.at_a_time}
              onChange={(e) => change({ at_a_time: Number(e.target.value) })}
            />
          </label>
          <label htmlFor="autopilot-daily" className="grid w-28 gap-1 text-sm">
            Per day
            <Input
              id="autopilot-daily"
              type="number"
              min={1}
              max={1000}
              value={draft.per_day}
              onChange={(e) => change({ per_day: Number(e.target.value) })}
            />
          </label>
          <label
            htmlFor="autopilot-reviewer"
            className="flex items-center gap-2 text-sm"
          >
            <Switch id="autopilot-reviewer" checked disabled />
            Reviewer required
          </label>
        </div>
        <p className="text-xs text-muted-foreground">
          Limits apply per agent; existing capacity can lower them. The daily
          limit is shared by this project.
        </p>
        <label
          htmlFor="autopilot-enabled"
          className="flex items-center gap-3 text-sm font-medium"
        >
          Autopilot
          <Switch
            id="autopilot-enabled"
            checked={Boolean(current?.record.payload.enabled)}
            onCheckedChange={(v) => void save(v)}
          />
        </label>
        <p className="text-sm" data-testid="autopilot-review-line">
          {autopilotReviewLine(draft, draft.agents.map(label))}
        </p>
        {dirty && current ? (
          <Button
            size="sm"
            onClick={() => void save(current.record.payload.enabled)}
          >
            Save Autopilot settings
          </Button>
        ) : null}
        {needsConfirmation ? (
          <div
            className="space-y-3 rounded-lg border border-border p-3"
            role="status"
          >
            <p>Autopilot made 3 assignments in the first hour. Keep it on?</p>
            <div className="flex gap-2">
              <Button onClick={() => void save(true, true)}>Keep on</Button>
              <Button variant="outline" onClick={() => void save(false)}>
                Pause
              </Button>
            </div>
          </div>
        ) : null}
      </fieldset>
      {error || matches.length > 1 ? (
        <p role="alert" className="text-sm text-destructive">
          {error ??
            "Multiple Autopilot records exist for this project. Resolve them before changing this setting."}{" "}
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() =>
              void save(
                retryIntent.current.enabled,
                retryIntent.current.confirm,
              )
            }
          >
            Retry save
          </Button>
        </p>
      ) : null}
      <div>
        <h4 className="text-sm font-semibold">What Autopilot has done</h4>
        {actions.length ? (
          <ul className="mt-2 space-y-2">
            {actions.map((action) => (
              <li key={action.eventId}>
                <button
                  type="button"
                  className="text-left text-sm text-primary hover:underline"
                  onClick={() => onOpenIssue(action.issueId)}
                >
                  {action.result} ·{" "}
                  {
                    context.issues.find((i) => i.issueId === action.issueId)
                      ?.title
                  }
                </button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="mt-2 text-sm text-muted-foreground">
            No assignments or alerts yet.
          </p>
        )}
      </div>
      <AssigneePickerDialog
        agentsOnly
        open={picker}
        onOpenChange={setPicker}
        assignee={null}
        disabled={busy}
        onAssign={(key) => {
          if (key && !draft.agents.includes(key))
            change({ agents: [...draft.agents, key] });
          setPicker(false);
        }}
      />
      <RelaySettingRecovery
        onRecovered={() => {
          setDirty(false);
          setError(null);
        }}
        stream={workspace}
        setting={`autopilot:${project}`}
        refresh={refresh}
      />
    </section>
  );
}
