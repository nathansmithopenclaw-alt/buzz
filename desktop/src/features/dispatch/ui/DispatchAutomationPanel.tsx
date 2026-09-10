import * as React from "react";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import {
  activeRecords,
  type AutomationControls,
  type RecordOf,
} from "../recordTypes";
import { publishPlanningRecord } from "../recordMutations";
import { automationPreview } from "../automationPreview";
import type { DispatchContext } from "../types";
import { DispatchCapacityEditor } from "./DispatchCapacityEditor";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/shared/ui/dialog";
import { AssigneePickerDialog } from "./AssigneePickerDialog";

const selectClass =
  "h-9 rounded border border-input bg-background px-2 text-sm";
export function DispatchAutomationPanel({
  context,
  workspace,
  viewer,
  refresh,
  onOpenIssue,
}: {
  context: DispatchContext;
  workspace: string;
  viewer: string;
  refresh: () => Promise<DispatchContext>;
  onOpenIssue: (id: string) => void;
}) {
  const services = activeRecords(context.records, "automation");
  const [editing, setEditing] = React.useState<AutomationControls | null>(null),
    [open, setOpen] = React.useState(false);
  const edit = (controls?: AutomationControls) => {
    setEditing(
      controls ?? {
        service_id: "triage-dispatcher",
        service_pubkey: "",
        mode: "preview",
        expires_at: null,
        max_actions_total: 3,
        max_actions_per_tick: 1,
        issue_ids: [],
        capture_channels: [],
        routing: [],
        notify_triage: false,
        unassigned_after_secs: null,
        notification_channel: workspace,
      },
    );
    setOpen(true);
  };
  return (
    <div
      className="mx-auto w-full max-w-5xl space-y-5 p-5"
      data-testid="dispatch-automation"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold">Automation</h1>
          <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
            Preview specific work, set a small allowance, and track what the
            service actually confirms. A paused or expired approval holds
            pending publications.
          </p>
        </div>
        <Button
          disabled={
            context.viewerRole !== "maintainer" ||
            Boolean(context.sectionErrors.records)
          }
          onClick={() => edit()}
        >
          Create preview
        </Button>
      </div>
      {context.sectionErrors.records ? (
        <p role="alert" className="text-sm text-destructive">
          Automation settings and observations are unavailable:{" "}
          {context.sectionErrors.records}
        </p>
      ) : null}
      {!services.length ? (
        <div className="rounded-xl border border-dashed border-border p-6">
          <h2 className="font-medium">No shared automation approvals</h2>
          <p className="mt-2 text-sm text-muted-foreground">
            Start with a preview and the identity of your configured dispatcher
            service. Dispatch will require a fresh service acknowledgement
            before offering activation.
          </p>
        </div>
      ) : (
        services.map((value) => (
          <ServiceCard
            key={value.record.record_id}
            value={value}
            context={context}
            workspace={workspace}
            refresh={refresh}
            onEdit={() => edit(value.record.payload.controls)}
            onOpenIssue={onOpenIssue}
          />
        ))
      )}
      <DispatchCapacityEditor
        context={context}
        workspace={workspace}
        refresh={refresh}
      />
      <section className="rounded-xl border border-border p-4">
        <h2 className="font-semibold">Recent confirmed service actions</h2>
        <p className="mt-1 text-xs text-muted-foreground">
          Signed receipts from configured service identities. An assignment
          receipt does not establish that the executor completed the work.
        </p>
        <div className="mt-3 space-y-2">
          {context.automation?.actions.length ? (
            context.automation.actions.map((action) => (
              <div
                key={action.eventId}
                className="flex flex-wrap items-center justify-between gap-2 border-t border-border py-2 text-sm"
              >
                <div>
                  <p>{action.result}</p>
                  <span className="text-xs text-muted-foreground">
                    {new Date(action.createdAt * 1000).toLocaleString()}
                  </span>
                </div>
                <Button
                  size="sm"
                  variant="link"
                  onClick={() => onOpenIssue(action.issueId)}
                >
                  Open issue
                </Button>
              </div>
            ))
          ) : (
            <p className="text-sm text-muted-foreground">
              No service action receipts observed.
            </p>
          )}
        </div>
      </section>
      {!context.admission ||
      context.admission.unsealed > 0 ||
      context.admission.diagnostics.length > 0 ? (
        <p className="text-sm text-muted-foreground" role="status">
          Activation is held until the relay supplies complete authorization
          history. Older records need an administrator to review and preserve a
          labeled legacy baseline. Preview and pause remain available.
        </p>
      ) : null}
      {editing ? (
        <AutomationEditor
          key={`${open}:${editing.service_id}:${viewer}`}
          open={open}
          onOpenChange={setOpen}
          initial={editing}
          context={context}
          workspace={workspace}
          refresh={refresh}
        />
      ) : null}
    </div>
  );
}
function ServiceCard({
  value,
  context,
  workspace,
  refresh,
  onEdit,
  onOpenIssue,
}: {
  value: RecordOf<"automation">;
  context: DispatchContext;
  workspace: string;
  refresh: () => Promise<DispatchContext>;
  onEdit: () => void;
  onOpenIssue: (id: string) => void;
}) {
  const controls = value.record.payload.controls,
    now = Date.now() / 1000;
  const [busy, setBusy] = React.useState(false),
    [error, setError] = React.useState<string | null>(null);
  const heartbeat = context.automation?.heartbeats.find(
    (beat) =>
      beat.serviceId === controls.service_id &&
      beat.signer === controls.service_pubkey,
  );
  let acknowledgement: {
    approval?: string;
    spent?: number;
    pending?: number;
    controls?: string;
  } | null = null;
  try {
    const parsed: unknown = JSON.parse(heartbeat?.health.detail ?? "null");
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      const candidate = parsed as Record<string, unknown>;
      acknowledgement = {
        approval:
          typeof candidate.approval === "string"
            ? candidate.approval
            : undefined,
        controls:
          typeof candidate.controls === "string"
            ? candidate.controls
            : undefined,
        spent:
          typeof candidate.spent === "number" &&
          Number.isSafeInteger(candidate.spent) &&
          candidate.spent >= 0
            ? candidate.spent
            : undefined,
        pending:
          typeof candidate.pending === "number" &&
          Number.isSafeInteger(candidate.pending) &&
          candidate.pending >= 0
            ? candidate.pending
            : undefined,
      };
    }
  } catch {
    /* Legacy or plain-language health remains visible below. */
  }
  const fresh = Boolean(
    heartbeat &&
      now - heartbeat.observedAt < 300 &&
      heartbeat.observedAt <= now + 300,
  );
  const acknowledged =
    fresh &&
    heartbeat?.health.status === "healthy" &&
    acknowledgement?.approval === value.event_id;
  const editable =
    context.viewerRole === "maintainer" && !context.sectionErrors.records;
  const preview = automationPreview(context, controls),
    expired = controls.expires_at !== null && controls.expires_at <= now;
  const mode = async (mode: AutomationControls["mode"]) => {
    setBusy(true);
    setError(null);
    try {
      await publishPlanningRecord({
        workspace,
        previous: value,
        payload: {
          type: "automation",
          controls: {
            ...controls,
            mode,
            expires_at:
              mode === "active"
                ? Math.floor(Date.now() / 1000) + 3600
                : controls.expires_at,
          },
        },
        refresh,
      });
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="rounded-xl border border-border p-4">
      <div className="flex flex-wrap justify-between gap-3">
        <div>
          <h2 className="font-semibold">{controls.service_id}</h2>
          <p className="mt-1 text-sm">
            {expired && controls.mode === "active"
              ? "Expired"
              : controls.mode === "active"
                ? "Bounded rollout"
                : controls.mode === "preview"
                  ? "Preview only"
                  : "Paused"}{" "}
            ·{" "}
            {heartbeat
              ? fresh
                ? acknowledged
                  ? "Service acknowledged"
                  : "Awaiting service acknowledgement"
                : "Service observation is stale"
              : "No service observed"}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={busy || !editable}
            onClick={onEdit}
          >
            Revise preview
          </Button>
          {controls.mode !== "paused" ? (
            <Button
              size="sm"
              variant="outline"
              disabled={busy || !editable}
              onClick={() => void mode("paused")}
            >
              Pause
            </Button>
          ) : (
            <Button
              size="sm"
              variant="outline"
              disabled={busy || !editable}
              onClick={() => void mode("preview")}
            >
              Preview
            </Button>
          )}
          {controls.mode === "preview" ? (
            <Button
              size="sm"
              disabled={
                busy ||
                !editable ||
                !acknowledged ||
                !context.admission ||
                context.admission.unsealed > 0 ||
                context.admission.diagnostics.length > 0 ||
                (!controls.issue_ids.length &&
                  !controls.capture_channels.length)
              }
              onClick={() => void mode("active")}
            >
              Activate for 1 hour
            </Button>
          ) : null}
        </div>
      </div>
      <p className="mt-3 text-sm text-muted-foreground">
        At most {controls.max_actions_total} distinct actions in this approval,{" "}
        {controls.max_actions_per_tick} per cycle. Restarting or pausing
        preserves the spent allowance.
      </p>
      {acknowledgement && fresh ? (
        <p className="mt-1 text-sm">
          Service reports {acknowledgement.spent ?? "unknown"} reserved actions
          and {acknowledgement.pending ?? "unknown"} pending publications.
        </p>
      ) : null}
      {heartbeat?.health.status === "degraded" ? (
        <p role="status" className="mt-2 text-sm text-warning">
          {heartbeat.health.detail}
        </p>
      ) : null}
      <details className="mt-4" open>
        <summary className="text-sm font-medium">
          Issue preview · {preview.filter((value) => value.ready).length}{" "}
          locally eligible
        </summary>
        <p className="mt-1 text-xs text-muted-foreground">
          The service checks current permissions, prior deliveries and remaining
          allowance before every action.
        </p>
        <ul className="mt-2 space-y-2">
          {preview.map((candidate) => (
            <li
              key={candidate.id}
              className="flex flex-wrap items-center justify-between gap-2 rounded bg-muted/20 p-2 text-sm"
            >
              <div>
                <button
                  type="button"
                  className="text-left font-medium hover:underline"
                  onClick={() => onOpenIssue(candidate.id)}
                >
                  {candidate.title}
                </button>
                <p className="text-xs text-muted-foreground">
                  {candidate.ready ? "Eligible" : "Held"} · {candidate.reason}
                </p>
              </div>
            </li>
          ))}
        </ul>
        {controls.capture_channels.length ? (
          <p className="mt-2 text-sm">
            New pins in {controls.capture_channels.length} explicitly selected
            conversation(s) may be captured if their audience permits it.
          </p>
        ) : null}
      </details>
      <details className="mt-3 text-xs text-muted-foreground">
        <summary>Service identity and observation</summary>
        <p className="mt-1 break-all">
          Expected signer: {controls.service_pubkey}
        </p>
        <p>
          {heartbeat
            ? `Last observation: ${new Date(heartbeat.observedAt * 1000).toLocaleString()}`
            : "A host must be configured separately with remote controls enabled."}
        </p>
      </details>
      {error ? (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </section>
  );
}
function AutomationEditor({
  open,
  onOpenChange,
  initial,
  context,
  workspace,
  refresh,
}: {
  open: boolean;
  onOpenChange: (value: boolean) => void;
  initial: AutomationControls;
  context: DispatchContext;
  workspace: string;
  refresh: () => Promise<DispatchContext>;
}) {
  const [draft, setDraft] = React.useState({
      ...initial,
      mode: "preview" as const,
      expires_at: null,
    }),
    [query, setQuery] = React.useState(""),
    [busy, setBusy] = React.useState(false),
    [error, setError] = React.useState<string | null>(null),
    [picker, setPicker] = React.useState<number | null>(null);
  const routeKeys = React.useRef(
    initial.routing.map(() => crypto.randomUUID()),
  );
  const { channels } = useChannelNavigation();
  const profiles = useUsersBatchQuery(
    draft.routing.map((route) => route.executor).filter(Boolean),
  );
  const label = (key: string) =>
    resolveUserLabel({ pubkey: key, profiles: profiles.data?.profiles });
  const preview = automationPreview(context, draft),
    update = <K extends keyof AutomationControls>(
      key: K,
      value: AutomationControls[K],
    ) => {
      if (key === "routing") {
        const routes = value as AutomationControls["routing"];
        if (routes.length !== draft.routing.length) {
          const previousKeys = routeKeys.current;
          routeKeys.current = routes.map((route) => {
            const index = draft.routing.indexOf(route);
            return index >= 0
              ? (previousKeys[index] ?? crypto.randomUUID())
              : crypto.randomUUID();
          });
        }
      }
      setDraft((current) => ({ ...current, [key]: value }));
    };
  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await publishPlanningRecord({
        workspace,
        payload: { type: "automation", controls: draft },
        refresh,
      });
      onOpenChange(false);
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!busy) onOpenChange(value);
      }}
    >
      <DialogContent className="max-h-[90vh] max-w-2xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Preview bounded automation</DialogTitle>
          <DialogDescription>
            A new preview replaces the previous service configuration and starts
            a separate allowance. Preview does not authorize assignments or
            notices.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={(event) => void save(event)}>
          <fieldset disabled={busy} className="space-y-4">
            <div className="grid gap-3 sm:grid-cols-2">
              <label
                htmlFor="automation-service-name"
                className="grid gap-1 text-sm"
              >
                Service name
                <Input
                  id="automation-service-name"
                  aria-label="Service name"
                  required
                  maxLength={64}
                  pattern="[a-z0-9_-]+"
                  value={draft.service_id}
                  onChange={(event) => update("service_id", event.target.value)}
                />
              </label>
              <label
                htmlFor="automation-action-limit"
                className="grid gap-1 text-sm"
              >
                Total action allowance
                <Input
                  id="automation-action-limit"
                  aria-label="Total action allowance"
                  type="number"
                  min={1}
                  max={50}
                  value={draft.max_actions_total}
                  onChange={(event) =>
                    update("max_actions_total", Number(event.target.value))
                  }
                />
              </label>
            </div>
            <label
              htmlFor="automation-service-key"
              className="grid gap-1 text-sm"
            >
              Expected service public key
              <Input
                id="automation-service-key"
                aria-label="Expected service public key"
                required
                pattern="[0-9a-f]{64}"
                value={draft.service_pubkey}
                onChange={(event) =>
                  update("service_pubkey", event.target.value.trim())
                }
              />
            </label>
            <label
              htmlFor="automation-cycle-limit"
              className="grid gap-1 text-sm"
            >
              Actions per cycle
              <Input
                id="automation-cycle-limit"
                aria-label="Actions per cycle"
                type="number"
                min={1}
                max={Math.min(10, draft.max_actions_total)}
                value={draft.max_actions_per_tick}
                onChange={(event) =>
                  update("max_actions_per_tick", Number(event.target.value))
                }
              />
            </label>
            <fieldset className="rounded border border-border p-3">
              <legend className="px-1 text-sm font-medium">
                Specific issues · {draft.issue_ids.length} of 50
              </legend>
              <Input
                aria-label="Find issues for automation"
                value={query}
                placeholder="Find work to include"
                onChange={(event) => setQuery(event.target.value)}
              />
              <div className="mt-2 max-h-40 space-y-2 overflow-y-auto">
                {context.issues
                  .filter(
                    (issue) =>
                      issue.title.toLowerCase().includes(query.toLowerCase()) ||
                      draft.issue_ids.includes(issue.issueId),
                  )
                  .slice(0, 100)
                  .map((issue) => (
                    <label
                      key={issue.issueId}
                      className="flex items-start gap-2 text-sm"
                    >
                      <input
                        type="checkbox"
                        checked={draft.issue_ids.includes(issue.issueId)}
                        disabled={
                          !draft.issue_ids.includes(issue.issueId) &&
                          draft.issue_ids.length >= 50
                        }
                        onChange={(event) =>
                          update(
                            "issue_ids",
                            event.target.checked
                              ? [...draft.issue_ids, issue.issueId]
                              : draft.issue_ids.filter(
                                  (id) => id !== issue.issueId,
                                ),
                          )
                        }
                      />
                      {issue.title}
                    </label>
                  ))}
              </div>
            </fieldset>
            <fieldset className="space-y-3 rounded border border-border p-3">
              <legend className="px-1 text-sm font-medium">
                Capability routes
              </legend>
              {draft.routing.map((route, index) => (
                <div
                  key={routeKeys.current[index]}
                  className="grid gap-2 sm:grid-cols-2"
                >
                  <Input
                    aria-label={`Route ${index + 1} label`}
                    placeholder="Capability label"
                    value={route.label}
                    onChange={(event) =>
                      update(
                        "routing",
                        draft.routing.map((route, i) =>
                          i === index
                            ? {
                                ...route,
                                label: event.target.value.trim().toLowerCase(),
                              }
                            : route,
                        ),
                      )
                    }
                  />
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => setPicker(index)}
                  >
                    {route.executor ? label(route.executor) : "Choose executor"}
                  </Button>
                  <select
                    className={selectClass}
                    aria-label={`Route ${index + 1} conversation`}
                    value={route.channel}
                    onChange={(event) =>
                      update(
                        "routing",
                        draft.routing.map((route, i) =>
                          i === index
                            ? { ...route, channel: event.target.value }
                            : route,
                        ),
                      )
                    }
                  >
                    {channels.map((channel) => (
                      <option key={channel.id} value={channel.id}>
                        {channel.name}
                      </option>
                    ))}
                  </select>
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={() =>
                      update(
                        "routing",
                        draft.routing.filter((_, i) => i !== index),
                      )
                    }
                  >
                    Remove route
                  </Button>
                </div>
              ))}
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={draft.routing.length >= 20}
                onClick={() =>
                  update("routing", [
                    ...draft.routing,
                    { label: "", executor: "", channel: workspace },
                  ])
                }
              >
                Add route
              </Button>
              <AssigneePickerDialog
                open={picker !== null}
                onOpenChange={(value) => {
                  if (!value) setPicker(null);
                }}
                assignee={
                  picker === null
                    ? null
                    : draft.routing[picker]?.executor || null
                }
                disabled={busy}
                onAssign={(executor) => {
                  if (picker !== null)
                    update(
                      "routing",
                      draft.routing.map((route, i) =>
                        i === picker
                          ? { ...route, executor: executor ?? "" }
                          : route,
                      ),
                    );
                  setPicker(null);
                }}
              />
            </fieldset>
            <details className="rounded border border-border p-3">
              <summary className="text-sm font-medium">
                Pin capture and human notices
              </summary>
              <p className="my-2 text-xs text-muted-foreground">
                Only pins created after this preview are eligible. Copying text
                requires the Dispatch audience to fit inside the source
                conversation.
              </p>
              <div className="max-h-32 space-y-2 overflow-y-auto">
                {channels.map((channel) => (
                  <label key={channel.id} className="flex gap-2 text-sm">
                    <input
                      type="checkbox"
                      checked={draft.capture_channels.includes(channel.id)}
                      disabled={
                        !draft.capture_channels.includes(channel.id) &&
                        draft.capture_channels.length >= 20
                      }
                      onChange={(event) =>
                        update(
                          "capture_channels",
                          event.target.checked
                            ? [...draft.capture_channels, channel.id]
                            : draft.capture_channels.filter(
                                (id) => id !== channel.id,
                              ),
                        )
                      }
                    />
                    {channel.name}
                  </label>
                ))}
              </div>
              <label className="mt-3 flex gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={draft.notify_triage}
                  onChange={(event) =>
                    update("notify_triage", event.target.checked)
                  }
                />
                Notify once for each unowned triage entry
              </label>
              <label className="mt-3 flex gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={draft.unassigned_after_secs !== null}
                  onChange={(event) =>
                    update(
                      "unassigned_after_secs",
                      event.target.checked ? 86400 : null,
                    )
                  }
                />
                Notify when selected work remains unowned for 24 hours
              </label>
              <select
                aria-label="Notice conversation"
                className={`${selectClass} mt-3 w-full`}
                value={draft.notification_channel}
                onChange={(event) =>
                  update("notification_channel", event.target.value)
                }
              >
                {channels.map((channel) => (
                  <option key={channel.id} value={channel.id}>
                    {channel.name}
                  </option>
                ))}
              </select>
            </details>
            <p className="text-sm">
              {preview.filter((value) => value.ready).length} of{" "}
              {preview.length} selected issues are locally eligible. The
              configured host may further restrict routes and channels.
            </p>
            {error ? (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            ) : null}
            <Button type="submit" disabled={busy}>
              {busy ? "Saving…" : "Save preview"}
            </Button>
          </fieldset>
        </form>
      </DialogContent>
    </Dialog>
  );
}
