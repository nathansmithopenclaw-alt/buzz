import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  managedAgentsQueryKey,
  useAcpRuntimesQuery,
  useCreateManagedAgentMutation,
  useCreatePersonaMutation,
  useManagedAgentsQuery,
  usePersonasQuery,
  useUpdatePersonaMutation,
} from "@/features/agents/hooks";
import { getDefaultPersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import {
  formatRuntimeOptionLabel,
  sortPersonaRuntimes,
} from "@/features/agents/ui/agentConfigOptions";
import { usePersonaModelDiscovery } from "@/features/agents/ui/usePersonaModelDiscovery";
import { useGlobalAgentConfig } from "@/features/agents/useGlobalAgentConfig";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  ensureNectarReporter,
  findNectarAgent,
  readNectarAvatarDataUrl,
} from "@/features/pulse/lib/nectarPersona";
import {
  configFromEvent,
  conversationsFromChannels,
  fetchBriefConfig,
  fetchBriefReports,
  publishBriefConfig,
  sourceConfigFromPayload,
  workflowIdFromEvent,
  type SummaryReport,
} from "@/features/pulse/lib/summaryProtocol";
import {
  defaultSourceConfig,
  type SummarySourceConfig,
} from "@/features/pulse/lib/summarySources";
import {
  SummaryBuilder,
  type SummaryReporterMode,
} from "@/features/pulse/ui/SummaryBuilder";
import type { AcpRuntime } from "@/shared/api/types";
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
  const personasQuery = usePersonasQuery();
  const runtimesQuery = useAcpRuntimesQuery();
  const { globalConfig } = useGlobalAgentConfig();
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const createAgentMutation = useCreateManagedAgentMutation();
  const conversations = React.useMemo(
    () => conversationsFromChannels(channelsQuery.data ?? []),
    [channelsQuery.data],
  );
  const nectarAgent = React.useMemo(
    () => findNectarAgent(agentsQuery.data ?? []),
    [agentsQuery.data],
  );

  const configQuery = useQuery({
    queryKey: ["brief-config", currentPubkey],
    enabled: Boolean(currentPubkey),
    queryFn: () => fetchBriefConfig(currentPubkey as string),
  });
  const payload = configQuery.data ? configFromEvent(configQuery.data) : null;
  const reportsQuery = useQuery({
    queryKey: [
      "brief-reports",
      currentPubkey,
      payload?.reporter_pubkey ?? null,
    ],
    enabled: Boolean(currentPubkey) && configQuery.isFetched,
    queryFn: () =>
      fetchBriefReports(
        currentPubkey as string,
        payload?.reporter_pubkey ?? null,
      ),
  });
  const hasSavedConfig = Boolean(payload);
  const showBuilder = !hasSavedConfig || editing;

  const [sourceConfig, setSourceConfig] = React.useState<SummarySourceConfig>(
    () => defaultSourceConfig(conversations),
  );
  const [hour, setHour] = React.useState(9);
  const [minute, setMinute] = React.useState(0);
  const [reporterMode, setReporterMode] =
    React.useState<SummaryReporterMode>("nectar");
  const [reporterPubkey, setReporterPubkey] = React.useState<string | null>(
    null,
  );
  const [runtimeId, setRuntimeId] = React.useState("");
  const [model, setModel] = React.useState("");

  const defaultRuntime = React.useMemo(
    () => getDefaultPersonaRuntime(runtimesQuery.data ?? []),
    [runtimesQuery.data],
  );
  const availableRuntimes = React.useMemo(
    () =>
      sortPersonaRuntimes(runtimesQuery.data ?? []).filter(
        (runtime): runtime is AcpRuntime =>
          runtime.availability === "available",
      ),
    [runtimesQuery.data],
  );
  const runtimeOptions = React.useMemo(
    () =>
      availableRuntimes.map((runtime) => ({
        id: runtime.id,
        label: formatRuntimeOptionLabel(runtime),
      })),
    [availableRuntimes],
  );
  const selectedRuntime = availableRuntimes.find(
    (runtime) => runtime.id === runtimeId,
  );

  React.useEffect(() => {
    if (!runtimeId && defaultRuntime) {
      setRuntimeId(defaultRuntime.id);
    }
  }, [defaultRuntime, runtimeId]);

  const modelDiscovery = usePersonaModelDiscovery({
    envVars: globalConfig.env_vars,
    isCustomProviderEditing: false,
    model: model || undefined,
    modelFieldVisible: showBuilder && reporterMode === "nectar" && !nectarAgent,
    open: showBuilder,
    provider: globalConfig.provider ?? "",
    selectedRuntime,
  });
  const modelOptions = React.useMemo(
    () =>
      (modelDiscovery.discoveredModelOptions ?? [])
        .filter((option) => option.id.length > 0)
        .map((option) => ({ id: option.id, label: option.label })),
    [modelDiscovery.discoveredModelOptions],
  );

  React.useEffect(() => {
    if (!payload) {
      setSourceConfig(defaultSourceConfig(conversations));
      setHour(9);
      setMinute(0);
      setReporterMode("nectar");
      setReporterPubkey(nectarAgent?.pubkey ?? null);
      return;
    }
    setSourceConfig(sourceConfigFromPayload(payload, conversations));
    setHour(payload.hour);
    setMinute(payload.minute);
    setReporterPubkey(payload.reporter_pubkey);
    const assignedNectar =
      Boolean(nectarAgent) &&
      payload.reporter_pubkey?.toLowerCase() ===
        nectarAgent?.pubkey.toLowerCase();
    setReporterMode(
      assignedNectar || !payload.reporter_pubkey ? "nectar" : "existing",
    );
  }, [conversations, nectarAgent, payload]);

  const saveMutation = useMutation({
    mutationFn: async () => {
      if (!currentPubkey) {
        throw new Error("Sign in to save Summary sources.");
      }
      let reporter = reporterPubkey;
      if (reporterMode === "nectar") {
        if (nectarAgent) {
          reporter = nectarAgent.pubkey;
        } else {
          if (!selectedRuntime) {
            throw new Error("Choose a harness for Nectar.");
          }
          reporter = await ensureNectarReporter({
            agents: agentsQuery.data ?? [],
            personas: personasQuery.data ?? [],
            runtime: selectedRuntime,
            model,
            provider: globalConfig.provider,
            avatarDataUrl: await readNectarAvatarDataUrl(),
            createPersona: (input) => createPersonaMutation.mutateAsync(input),
            updatePersona: (input) => updatePersonaMutation.mutateAsync(input),
            createManagedAgent: (input) =>
              createAgentMutation.mutateAsync(input),
          });
        }
      }
      if (!reporter) {
        throw new Error(
          "Choose Nectar's harness, or pick an existing colleague.",
        );
      }
      const reporterAgent = (agentsQuery.data ?? []).find(
        (agent) => agent.pubkey.toLowerCase() === reporter.toLowerCase(),
      );
      await publishBriefConfig({
        ownerPubkey: currentPubkey,
        hour,
        minute,
        timezone: payload?.timezone || timezone,
        autoInclude: sourceConfig.autoInclude,
        reporterPubkey: reporter,
        reporterName: reporterAgent?.name || "Nectar",
        conversations,
        sourceConfig,
        previousChannelIds: payload?.channels ?? [],
        previousWorkflowId: configQuery.data
          ? workflowIdFromEvent(configQuery.data)
          : null,
      });
    },
    onSuccess: async () => {
      setEditing(false);
      await queryClient.invalidateQueries({ queryKey: ["brief-config"] });
      await queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
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
        model={model}
        modelLoading={modelDiscovery.modelDiscoveryLoading}
        modelOptions={modelOptions}
        nectarExists={Boolean(nectarAgent)}
        onCancel={hasSavedConfig ? () => setEditing(false) : undefined}
        onModelChange={setModel}
        onReporterChange={setReporterPubkey}
        onReporterModeChange={setReporterMode}
        onRuntimeChange={(nextRuntimeId) => {
          setRuntimeId(nextRuntimeId);
          setModel("");
        }}
        onSave={() => saveMutation.mutate()}
        onSourceConfigChange={setSourceConfig}
        onTimeChange={(nextHour, nextMinute) => {
          setHour(nextHour);
          setMinute(nextMinute);
        }}
        reporterMode={reporterMode}
        reporterPubkey={reporterPubkey}
        runtimeId={runtimeId}
        runtimes={runtimeOptions}
        runtimesLoading={runtimesQuery.isLoading}
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
