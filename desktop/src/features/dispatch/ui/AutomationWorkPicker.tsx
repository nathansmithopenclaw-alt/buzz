import * as React from "react";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { activeRecords, type AutomationControls } from "../recordTypes";
import type { AutomationTask } from "../automationPresentation";
import type { DispatchContext } from "../types";

const selectClass =
  "h-10 w-full rounded-md border border-input bg-background px-3 text-sm";
export function AutomationWorkPicker({
  context,
  controls,
  selected,
  workspace,
  update,
}: {
  context: DispatchContext;
  controls: AutomationControls;
  selected: AutomationTask[];
  workspace: string;
  update: <K extends keyof AutomationControls>(
    key: K,
    value: AutomationControls[K],
  ) => void;
}) {
  const { channels } = useChannelNavigation();
  const [issueQuery, setIssueQuery] = React.useState(""),
    [channelQuery, setChannelQuery] = React.useState("");
  const agents = activeRecords(context.records, "capacity");
  const profiles = useUsersBatchQuery(
    [
      ...agents.map((agent) => agent.record.payload.executor),
      ...controls.routing.map((route) => route.executor),
    ].filter(Boolean),
  );
  const agentName = (key: string) =>
    resolveUserLabel({
      pubkey: key,
      profiles: profiles.data?.profiles,
      fallbackName: "Agent name unavailable",
    });
  const labels = [
    ...new Set([
      ...context.issues.flatMap((issue) => issue.labels),
      ...agents.flatMap((agent) => agent.record.payload.capabilities),
      ...controls.routing.map((route) => route.label).filter(Boolean),
    ]),
  ].sort();
  const routeKeys = React.useRef(
    controls.routing.map(() => crypto.randomUUID()),
  );
  while (routeKeys.current.length < controls.routing.length)
    routeKeys.current.push(crypto.randomUUID());
  const changeRoute = (
    index: number,
    key: keyof AutomationControls["routing"][number],
    value: string,
  ) =>
    update(
      "routing",
      controls.routing.map((route, i) =>
        i === index ? { ...route, [key]: value } : route,
      ),
    );
  const pick = (
    key: "capture_channels" | "issue_ids",
    id: string,
    checked: boolean,
  ) =>
    update(
      key,
      checked
        ? [...controls[key], id]
        : controls[key].filter((value) => value !== id),
    );
  const channelOptions = (value: string) => (
    <>
      {!channels.some((channel) => channel.id === value) ? (
        <option value={value}>Channel unavailable — choose another</option>
      ) : null}
      {channels.map((channel) => (
        <option key={channel.id} value={channel.id}>
          #{channel.name}
        </option>
      ))}
    </>
  );
  const visibleIssues = context.issues
    .filter(
      (issue) =>
        controls.issue_ids.includes(issue.issueId) ||
        (!["done", "cancelled"].includes(issue.status) &&
          `${issue.title} ${issue.identifier}`
            .toLowerCase()
            .includes(issueQuery.toLowerCase())),
    )
    .sort(
      (a, b) =>
        Number(controls.issue_ids.includes(b.issueId)) -
        Number(controls.issue_ids.includes(a.issueId)),
    );
  return (
    <>
      {selected.includes("capture") ? (
        <section
          className="space-y-3 rounded-xl border border-border p-4"
          aria-label="Message channels"
        >
          <h4 className="text-sm font-semibold">
            Which channels should Buzz watch?
          </h4>
          <p className="text-sm text-muted-foreground">
            Add the 📌 emoji reaction to a message in one of these channels to
            make it an issue.
          </p>
          <Input
            aria-label="Find channels"
            placeholder="Search channels"
            value={channelQuery}
            onChange={(event) => setChannelQuery(event.target.value)}
          />
          <div className="max-h-44 space-y-2 overflow-y-auto">
            {channels
              .filter(
                (channel) =>
                  channel.name
                    .toLowerCase()
                    .includes(channelQuery.toLowerCase()) ||
                  controls.capture_channels.includes(channel.id),
              )
              .map((channel) => (
                <label
                  key={channel.id}
                  className="flex items-center gap-2 rounded p-1 text-sm hover:bg-muted/40"
                >
                  <input
                    type="checkbox"
                    className="size-4 accent-primary"
                    checked={controls.capture_channels.includes(channel.id)}
                    disabled={
                      !controls.capture_channels.includes(channel.id) &&
                      controls.capture_channels.length >= 20
                    }
                    onChange={(event) =>
                      pick("capture_channels", channel.id, event.target.checked)
                    }
                  />
                  #{channel.name}
                </label>
              ))}
            {controls.capture_channels
              .filter((id) => !channels.some((channel) => channel.id === id))
              .map((id) => (
                <label key={id} className="flex gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked
                    onChange={() => pick("capture_channels", id, false)}
                  />
                  Unavailable channel — uncheck to remove
                </label>
              ))}
            {!channels.length ? (
              <p className="text-sm text-muted-foreground">
                Join a channel to include its messages here.
              </p>
            ) : null}
          </div>
          <p className="text-xs text-muted-foreground">
            {controls.capture_channels.length} of 20 channels selected. Messages
            stay private if this workspace has a wider audience.
          </p>
        </section>
      ) : null}
      {selected.includes("assign") || selected.includes("remind") ? (
        <section
          className="space-y-3 rounded-xl border border-border p-4"
          aria-label="Selected issues"
        >
          <h4 className="text-sm font-semibold">
            Which issues should Buzz follow up on?
          </h4>
          <p className="text-sm text-muted-foreground">
            Choose specific work for this run. New issues are not added
            automatically.
          </p>
          <Input
            aria-label="Find issues for automation"
            placeholder="Search issues by title or number"
            value={issueQuery}
            onChange={(event) => setIssueQuery(event.target.value)}
          />
          <div className="max-h-48 space-y-2 overflow-y-auto">
            {visibleIssues.slice(0, 100).map((issue) => (
              <label
                key={issue.issueId}
                className="flex items-start gap-2 rounded p-1 text-sm hover:bg-muted/40"
              >
                <input
                  type="checkbox"
                  className="mt-0.5 size-4 shrink-0 accent-primary"
                  checked={controls.issue_ids.includes(issue.issueId)}
                  disabled={
                    !controls.issue_ids.includes(issue.issueId) &&
                    controls.issue_ids.length >= 50
                  }
                  onChange={(event) =>
                    pick("issue_ids", issue.issueId, event.target.checked)
                  }
                />
                {issue.title}
              </label>
            ))}
            {controls.issue_ids
              .filter(
                (id) => !context.issues.some((issue) => issue.issueId === id),
              )
              .map((id) => (
                <label key={id} className="flex gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked
                    onChange={() => pick("issue_ids", id, false)}
                  />
                  Unavailable issue — uncheck to remove
                </label>
              ))}
            {!visibleIssues.length ? (
              <p className="text-sm text-muted-foreground">
                No matching open issues. Try another search.
              </p>
            ) : null}
          </div>
          <p className="text-xs text-muted-foreground">
            {controls.issue_ids.length} of 50 issues selected.
            {visibleIssues.length > 100 ? " Search to narrow the list." : ""}
          </p>
        </section>
      ) : null}
      {selected.includes("remind") ? (
        <section
          className="space-y-3 rounded-xl border border-border p-4"
          aria-label="Reminders"
        >
          <h4 className="text-sm font-semibold">
            When should Buzz send a reminder?
          </h4>
          <label className="flex items-start gap-2 text-sm">
            <input
              type="checkbox"
              className="mt-0.5 size-4 accent-primary"
              checked={controls.notify_triage}
              onChange={(event) =>
                update("notify_triage", event.target.checked)
              }
            />
            When a selected issue is in Triage without an owner
          </label>
          <label className="flex items-start gap-2 text-sm">
            <input
              type="checkbox"
              className="mt-0.5 size-4 accent-primary"
              checked={controls.unassigned_after_secs !== null}
              onChange={(event) =>
                update(
                  "unassigned_after_secs",
                  event.target.checked ? 86400 : null,
                )
              }
            />
            When a selected issue has been waiting for an owner
          </label>
          {controls.unassigned_after_secs !== null ? (
            <label
              className="grid max-w-48 gap-1 text-sm"
              htmlFor="automation-wait-hours"
            >
              Waiting time in hours
              <Input
                id="automation-wait-hours"
                type="number"
                min={1}
                max={720}
                value={controls.unassigned_after_secs / 3600}
                onChange={(event) =>
                  update(
                    "unassigned_after_secs",
                    Number(event.target.value) * 3600,
                  )
                }
              />
            </label>
          ) : null}
          <label className="grid gap-1 text-sm">
            Post reminders in
            <select
              className={selectClass}
              value={controls.notification_channel}
              onChange={(event) =>
                update("notification_channel", event.target.value)
              }
            >
              {channelOptions(controls.notification_channel)}
            </select>
          </label>
          <p className="text-xs text-muted-foreground">
            Buzz checks only the selected issues and avoids repeating the same
            reminder.
          </p>
        </section>
      ) : null}
      {selected.includes("assign") ? (
        <section
          className="space-y-3 rounded-xl border border-border p-4"
          aria-label="Agent assignments"
        >
          <h4 className="text-sm font-semibold">Who should take the work?</h4>
          <p className="text-sm text-muted-foreground">
            Match an issue label to an agent. Buzz sends only selected issues
            that are ready in Todo; the owner still reviews the result.
          </p>
          {!agents.length ? (
            <p className="rounded-lg bg-muted/40 p-3 text-sm">
              No agents have been set up to take work yet. Use “Add an agent”
              below, then choose the agent here.
            </p>
          ) : null}
          {controls.routing.map((route, index) => (
            <fieldset
              key={routeKeys.current[index]}
              className="space-y-3 rounded-lg bg-muted/30 p-3"
            >
              <legend className="px-1 text-xs font-medium">
                Assignment {index + 1}
              </legend>
              <label className="grid gap-1 text-sm">
                Issue label
                <select
                  aria-label={`Assignment ${index + 1} label`}
                  className={selectClass}
                  value={route.label}
                  onChange={(event) =>
                    changeRoute(index, "label", event.target.value)
                  }
                >
                  <option value="">Choose a label</option>
                  {labels.map((label) => (
                    <option key={label} value={label}>
                      {label}
                    </option>
                  ))}
                </select>
              </label>
              <label className="grid gap-1 text-sm">
                Send to
                <select
                  aria-label={`Assignment ${index + 1} agent`}
                  className={selectClass}
                  value={route.executor}
                  onChange={(event) =>
                    changeRoute(index, "executor", event.target.value)
                  }
                >
                  <option value="">Choose an agent</option>
                  {route.executor &&
                  !agents.some(
                    (agent) => agent.record.payload.executor === route.executor,
                  ) ? (
                    <option value={route.executor}>
                      {agentName(route.executor)} — setup needed
                    </option>
                  ) : null}
                  {agents.map((agent) => (
                    <option
                      key={agent.event_id}
                      value={agent.record.payload.executor}
                    >
                      {agentName(agent.record.payload.executor)}
                      {agent.record.payload.paused ? " (paused)" : ""}
                    </option>
                  ))}
                </select>
              </label>
              <label className="grid gap-1 text-sm">
                Post the assignment in
                <select
                  aria-label={`Assignment ${index + 1} channel`}
                  className={selectClass}
                  value={route.channel}
                  onChange={(event) =>
                    changeRoute(index, "channel", event.target.value)
                  }
                >
                  {channelOptions(route.channel)}
                </select>
              </label>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  routeKeys.current.splice(index, 1);
                  update(
                    "routing",
                    controls.routing.filter((_, i) => i !== index),
                  );
                }}
              >
                Remove assignment {index + 1}
              </Button>
            </fieldset>
          ))}
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={controls.routing.length >= 20}
            onClick={() =>
              update("routing", [
                ...controls.routing,
                { label: "", executor: "", channel: workspace },
              ])
            }
          >
            Add another assignment rule
          </Button>
        </section>
      ) : null}
    </>
  );
}
