import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  configFromEvent,
  conversationsFromChannels,
  fetchBriefConfig,
  fetchBriefReports,
  publishBriefConfig,
  sourceConfigFromPayload,
  type SummaryReport,
} from "@/features/pulse/lib/summaryProtocol";
import {
  defaultSourceConfig,
  type SummarySourceConfig,
} from "@/features/pulse/lib/summarySources";
import { SummaryBuilder } from "@/features/pulse/ui/SummaryBuilder";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { Skeleton } from "@/shared/ui/skeleton";

type SummaryViewProps = {
  currentPubkey?: string;
};

function localTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

function formatReportWhen(createdAt: number): string {
  return new Date(createdAt * 1000).toLocaleString(undefined, {
    weekday: "long",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function SummaryView({ currentPubkey }: SummaryViewProps) {
  const queryClient = useQueryClient();
  const timezone = React.useMemo(() => localTimezone(), []);
  const [editing, setEditing] = React.useState(false);
  const channelsQuery = useChannelsQuery();
  const agentsQuery = useManagedAgentsQuery();
  const conversations = React.useMemo(
    () => conversationsFromChannels(channelsQuery.data ?? []),
    [channelsQuery.data],
  );

  const configQuery = useQuery({
    queryKey: ["brief-config", currentPubkey],
    enabled: Boolean(currentPubkey),
    queryFn: () => fetchBriefConfig(currentPubkey as string),
  });
  const reportsQuery = useQuery({
    queryKey: ["brief-reports", currentPubkey],
    enabled: Boolean(currentPubkey),
    queryFn: () => fetchBriefReports(currentPubkey as string),
  });

  const payload = configQuery.data ? configFromEvent(configQuery.data) : null;
  const hasSavedConfig = Boolean(payload);
  const showBuilder = !hasSavedConfig || editing;

  const [sourceConfig, setSourceConfig] = React.useState<SummarySourceConfig>(
    () => defaultSourceConfig(conversations),
  );
  const [hour, setHour] = React.useState(9);
  const [minute, setMinute] = React.useState(0);
  const [reporterPubkey, setReporterPubkey] = React.useState<string | null>(
    null,
  );

  React.useEffect(() => {
    if (!payload) {
      setSourceConfig(defaultSourceConfig(conversations));
      setHour(9);
      setMinute(0);
      const synopsis = (agentsQuery.data ?? []).find(
        (agent) => agent.name.toLowerCase() === "synopsis",
      );
      setReporterPubkey(synopsis?.pubkey ?? null);
      return;
    }
    setSourceConfig(sourceConfigFromPayload(payload, conversations));
    setHour(payload.hour);
    setMinute(payload.minute);
    setReporterPubkey(payload.reporter_pubkey);
  }, [agentsQuery.data, conversations, payload]);

  const saveMutation = useMutation({
    mutationFn: async () => {
      if (!currentPubkey) {
        throw new Error("Sign in to save Summary sources.");
      }
      await publishBriefConfig({
        ownerPubkey: currentPubkey,
        hour,
        minute,
        timezone: payload?.timezone || timezone,
        autoInclude: sourceConfig.autoInclude,
        reporterPubkey,
        conversations,
        sourceConfig,
      });
    },
    onSuccess: async () => {
      setEditing(false);
      await queryClient.invalidateQueries({ queryKey: ["brief-config"] });
    },
  });

  const reports: SummaryReport[] = reportsQuery.data ?? [];
  const latest = reports[0] ?? null;
  const older = reports.slice(1);

  if (configQuery.isLoading || channelsQuery.isLoading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-8 w-48" />
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-24 w-full" />
      </div>
    );
  }

  if (showBuilder) {
    return (
      <SummaryBuilder
        agents={agentsQuery.data ?? []}
        conversations={conversations}
        error={
          saveMutation.error instanceof Error
            ? saveMutation.error.message
            : saveMutation.error
              ? "Failed to save Summary sources"
              : null
        }
        hour={hour}
        minute={minute}
        onCancel={hasSavedConfig ? () => setEditing(false) : undefined}
        onReporterChange={setReporterPubkey}
        onSave={() => saveMutation.mutate()}
        onSourceConfigChange={setSourceConfig}
        onTimeChange={(nextHour, nextMinute) => {
          setHour(nextHour);
          setMinute(nextMinute);
        }}
        reporterPubkey={reporterPubkey}
        saving={saveMutation.isPending}
        sourceConfig={sourceConfig}
        timezone={payload?.timezone || timezone}
      />
    );
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold tracking-tight">
            {latest?.report.title ?? "Summary"}
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {latest
              ? formatReportWhen(latest.createdAt)
              : "No reports yet. The next run writes here."}
          </p>
        </div>
        <Button
          data-testid="summary-sources"
          onClick={() => setEditing(true)}
          size="sm"
          type="button"
          variant="ghost"
        >
          Sources
        </Button>
      </div>

      {latest ? (
        <article className="rounded-2xl border border-border/60 bg-background/70 px-4 py-4">
          <Markdown className="text-sm" content={latest.report.markdown} />
        </article>
      ) : (
        <div className="rounded-2xl border border-dashed border-border/60 px-4 py-12 text-center text-sm text-muted-foreground">
          Sources are saved. Pick a reporter and it will publish the first
          report at {String(hour).padStart(2, "0")}:
          {String(minute).padStart(2, "0")} ({payload?.timezone || timezone}).
        </div>
      )}

      {older.length > 0 ? (
        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-muted-foreground/70">
            Earlier
          </h3>
          {older.map((report) => (
            <details
              className="rounded-xl border border-border/50 bg-background/60 px-4 py-3"
              key={report.id}
            >
              <summary className="cursor-pointer text-sm font-medium">
                {report.report.title}
                <span className="ml-2 font-normal text-muted-foreground">
                  {formatReportWhen(report.createdAt)}
                </span>
              </summary>
              <div className="mt-3">
                <Markdown
                  className="text-sm"
                  content={report.report.markdown}
                />
              </div>
            </details>
          ))}
        </section>
      ) : null}
    </div>
  );
}
