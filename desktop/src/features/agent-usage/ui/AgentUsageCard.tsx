import * as React from "react";

import type {
  AgentUsage,
  AgentUsageChannel,
  ReportedUsage,
} from "@/shared/api/tauriArchive";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";

import { useAgentUsageSeries } from "../hooks";
import {
  cacheHitRate,
  costDisplay,
  isTeardownCause,
  peakContextSummary,
  provenanceRatio,
  sessionCauseLabel,
  totalSessions,
  usageDisplay,
  UNKNOWN_PLACEHOLDER,
  type UsageDisplay,
} from "../format";

const RANGES = [
  { days: 1, label: "24h" },
  { days: 7, label: "7d" },
  { days: 30, label: "30d" },
] as const;

/**
 * One usage figure.
 *
 * A value the archive does not know renders as a dash, never as `0` — the two
 * are different claims and the whole accounting layer is built to keep them
 * apart. A partial total is marked so it is never read as exact.
 */
function Figure({ label, display }: { label: string; display: UsageDisplay }) {
  return (
    <div className="min-w-0">
      <p className="text-xs text-muted-foreground/70">{label}</p>
      <p className="truncate text-sm font-medium tabular-nums">
        {display.text === null ? (
          <span
            className="text-muted-foreground/60"
            title="No event in this window reported this field. Unknown, not zero."
          >
            {UNKNOWN_PLACEHOLDER}
          </span>
        ) : (
          <>
            {display.partial ? (
              <span
                className="text-muted-foreground/60"
                title="At least one event did not report this field, so this is a reported lower bound, not a total."
              >
                ≥{" "}
              </span>
            ) : null}
            {display.text}
          </>
        )}
      </p>
    </div>
  );
}

function UsageFigures({ usage }: { usage: ReportedUsage }) {
  const rate = cacheHitRate(usage.inputTokens, usage.cacheReadTokens);
  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
      <Figure
        display={usageDisplay(usage.inputTokens)}
        label="Input (inclusive)"
      />
      <Figure
        display={usageDisplay(usage.cacheReadTokens)}
        label="Cache read"
      />
      <Figure
        display={usageDisplay(usage.cacheWriteTokens)}
        label="Cache write"
      />
      <Figure
        display={usageDisplay(usage.freshInputTokens)}
        label="Fresh input"
      />
      <Figure display={usageDisplay(usage.outputTokens)} label="Output" />
      <Figure
        display={costDisplay(usage.estimatedCostUsd)}
        label="Estimated cost"
      />
      {rate !== null ? (
        <Figure
          display={{ text: rate, partial: false }}
          label="Cache-read share"
        />
      ) : null}
    </div>
  );
}

/** Horizontal bar row. `max` of 0 renders an empty track rather than dividing by zero. */
function BarRow({
  label,
  value,
  max,
  hint,
}: {
  label: string;
  value: number;
  max: number;
  hint?: string;
}) {
  const pct = max > 0 ? Math.round((value / max) * 100) : 0;
  return (
    <div className="flex items-center gap-3" title={hint}>
      <span className="w-40 shrink-0 truncate text-xs text-muted-foreground/70">
        {label}
      </span>
      <span className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-muted">
        <span
          className="block h-full rounded-full bg-primary/60"
          style={{ width: `${pct}%` }}
        />
      </span>
      <span className="w-16 shrink-0 text-right text-xs tabular-nums">
        {value.toLocaleString("en-US")}
      </span>
    </div>
  );
}

function channelLabel(
  channel: AgentUsageChannel,
  nameById: Map<string, string>,
): string {
  if (channel.channelId === null) return "No channel (heartbeat)";
  return nameById.get(channel.channelId) ?? channel.channelId.slice(0, 8);
}

function AgentSection({
  agent,
  channelNameById,
  agentNameByPubkey,
}: {
  agent: AgentUsage;
  channelNameById: Map<string, string>;
  agentNameByPubkey: Map<string, string>;
}) {
  const name = agentNameByPubkey.get(agent.agentPubkey);
  const maxBucket = agent.buckets.reduce(
    (max, b) => Math.max(max, b.reportCount),
    0,
  );
  const teardowns = agent.sessionStarts.filter((s) => isTeardownCause(s.cause));
  const retries = provenanceRatio(
    agent.provenance.retriedTurns,
    agent.provenance.turnsWithAttempt,
  );
  const replays = provenanceRatio(
    agent.provenance.replayTurns,
    agent.provenance.turnsWithReplayFlag,
  );
  const peakContext = peakContextSummary(agent.provenance);

  return (
    <div
      className="space-y-4 border-t border-border/50 py-4 first:border-t-0 first:pt-0"
      data-agent-pubkey={agent.agentPubkey}
      data-testid="agent-usage-agent"
    >
      <div className="flex items-baseline justify-between gap-3">
        <p className="truncate text-sm font-medium">
          {name ?? truncatePubkey(agent.agentPubkey)}
        </p>
        <p className="shrink-0 text-xs text-muted-foreground/70 tabular-nums">
          {agent.reportCount.toLocaleString("en-US")} turns
        </p>
      </div>

      <UsageFigures usage={agent.usage} />

      {agent.buckets.length > 1 ? (
        <div className="space-y-1.5">
          <p className="text-xs font-medium text-muted-foreground/70">
            Turns per day
          </p>
          {agent.buckets.map((bucket) => (
            <BarRow
              key={bucket.start}
              label={new Date(bucket.start * 1000).toLocaleDateString("en-US", {
                month: "short",
                day: "numeric",
              })}
              max={maxBucket}
              value={bucket.reportCount}
            />
          ))}
        </div>
      ) : null}

      {agent.channels.length > 0 ? (
        <div className="space-y-1.5">
          <p className="text-xs font-medium text-muted-foreground/70">
            Turns per channel
          </p>
          {agent.channels.map((channel) => (
            <BarRow
              hint={channel.channelId ?? undefined}
              key={channel.channelId ?? "__none__"}
              label={channelLabel(channel, channelNameById)}
              max={agent.channels.reduce(
                (max, c) => Math.max(max, c.reportCount),
                0,
              )}
              value={channel.reportCount}
            />
          ))}
        </div>
      ) : null}

      {agent.sessionStarts.length > 0 ? (
        <div className="space-y-1.5">
          <p className="text-xs font-medium text-muted-foreground/70">
            Sessions started (
            {totalSessions(agent.sessionStarts).toLocaleString("en-US")})
          </p>
          {agent.sessionStarts.map((start) => (
            <BarRow
              hint={
                isTeardownCause(start.cause)
                  ? "Also the reason the previous session in that scope ended."
                  : "No predecessor session — nothing was torn down."
              }
              key={start.cause ?? "__unreported__"}
              label={sessionCauseLabel(start.cause)}
              max={agent.sessionStarts.reduce(
                (max, s) => Math.max(max, s.sessions),
                0,
              )}
              value={start.sessions}
            />
          ))}
          <p className="text-xs text-muted-foreground/60">
            {teardowns.length > 0
              ? `${totalSessions(teardowns).toLocaleString("en-US")} of these replaced a torn-down session.`
              : "None of these replaced a torn-down session."}
          </p>
        </div>
      ) : null}

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        <Figure
          display={{ text: retries, partial: false }}
          label="Retried turns"
        />
        <Figure
          display={{ text: replays, partial: false }}
          label="Startup-replay turns"
        />
        <Figure
          display={{ text: peakContext, partial: false }}
          label="Peak context"
        />
      </div>
      {retries === null && replays === null ? (
        <p className="text-xs text-muted-foreground/60">
          No turn in this window reported retry or replay provenance — that is
          unknown, not zero. Agents publish it from the harness build that
          introduced it onward.
        </p>
      ) : null}
    </div>
  );
}

/**
 * Read-only view of the locally archived NIP-AM turn metrics.
 *
 * Every figure comes from `getAgentUsageSeries`; this component performs no
 * accounting of its own beyond formatting. It never writes, and it never
 * queries the relay — the archive is the only source.
 */
export function AgentUsageCard() {
  const [days, setDays] = React.useState<number>(7);
  const seriesQuery = useAgentUsageSeries(days);
  const channelsQuery = useChannelsQuery();
  const agentsQuery = useManagedAgentsQuery();

  const channelNameById = React.useMemo(() => {
    const map = new Map<string, string>();
    for (const channel of channelsQuery.data ?? [])
      map.set(channel.id, channel.name);
    return map;
  }, [channelsQuery.data]);

  const agentNameByPubkey = React.useMemo(() => {
    const map = new Map<string, string>();
    for (const agent of agentsQuery.data ?? [])
      map.set(agent.pubkey, agent.name);
    return map;
  }, [agentsQuery.data]);

  const series = seriesQuery.data;

  return (
    <section className="min-w-0" data-testid="settings-agent-usage">
      <SettingsSectionHeader
        description={
          <>
            Token usage your agents reported for each completed turn, read from
            this device&rsquo;s local archive. Nothing here is fetched from the
            relay, and nothing is written.
          </>
        }
        title="Agent usage"
      />

      <div className="mb-4 flex gap-2">
        {RANGES.map((range) => (
          <Button
            data-testid={`agent-usage-range-${range.label}`}
            key={range.days}
            onClick={() => setDays(range.days)}
            size="sm"
            variant={days === range.days ? "default" : "outline"}
          >
            {range.label}
          </Button>
        ))}
      </div>

      <SettingsOptionGroup title="Reported usage">
        {seriesQuery.isPending ? (
          <SettingsOptionRow>
            <p className="text-sm text-muted-foreground/70">Loading…</p>
          </SettingsOptionRow>
        ) : seriesQuery.isError ? (
          <SettingsOptionRow>
            <p
              className="text-sm text-destructive"
              data-testid="agent-usage-error"
            >
              Could not read the usage archive: {String(seriesQuery.error)}
            </p>
          </SettingsOptionRow>
        ) : !series ? null : !series.collectionEnabled ? (
          <SettingsOptionRow>
            <p className="text-sm text-muted-foreground/70">
              Turn-metric archiving is off. Enable &ldquo;My agents&rsquo; turn
              metrics&rdquo; under Local archive to start collecting.
            </p>
          </SettingsOptionRow>
        ) : series.agents.length === 0 ? (
          <SettingsOptionRow>
            <p
              className="text-sm text-muted-foreground/70"
              data-testid="agent-usage-empty"
            >
              No turn metrics archived in this window.
            </p>
          </SettingsOptionRow>
        ) : (
          <div className="px-4 py-2">
            {series.agents.map((agent) => (
              <AgentSection
                agent={agent}
                agentNameByPubkey={agentNameByPubkey}
                channelNameById={channelNameById}
                key={agent.agentPubkey}
              />
            ))}
          </div>
        )}
      </SettingsOptionGroup>

      {series ? (
        <p
          className="mt-3 text-xs text-muted-foreground/60"
          data-testid="agent-usage-coverage"
        >
          {series.coverage.reportCount.toLocaleString("en-US")} turn metrics in
          this window
          {series.coverage.invalidReportCount > 0
            ? `, ${series.coverage.invalidReportCount.toLocaleString("en-US")} unparseable`
            : ""}
          .
          {series.coverage.hasUnknownUsage
            ? " Some fields were not reported, so totals marked ≥ are lower bounds."
            : ""}
        </p>
      ) : null}
    </section>
  );
}
