import { getChannelMembers } from "@/shared/api/tauriChannels";
import { useCommunities } from "@/features/communities/useCommunities";
import { Input } from "@/shared/ui/input";
import { SummaryAudienceFields } from "./SummaryAudienceFields";
import {
  saveNamedSummary,
  type SummaryDefinitionRecord,
  type NamedSummary,
  type SummaryAudience,
} from "../lib/namedSummaries";
import { listsFromChecked } from "../lib/summarySources";
import type { SummarySchedule } from "../lib/summarySchedule";
import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  managedAgentsQueryKey,
  useCreateManagedAgentMutation,
  useCreatePersonaMutation,
  useManagedAgentsQuery,
  usePersonasQuery,
  useUpdatePersonaMutation,
} from "@/features/agents/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { SummaryRuntimeDraft } from "./SummaryRuntimeFields";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  ensureNectarReporter,
  findNectarAgent,
  readNectarAvatarDataUrl,
} from "@/features/pulse/lib/nectarPersona";
import { conversationsFromChannels } from "@/features/pulse/lib/summaryProtocol";
import {
  defaultSourceConfig,
  type SummarySourceConfig,
} from "@/features/pulse/lib/summarySources";
import {
  SummaryBuilder,
  type SummaryReporterMode,
} from "@/features/pulse/ui/SummaryBuilder";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";

type SummaryViewProps = {
  currentPubkey?: string;
  record?: SummaryDefinitionRecord;
  draftId?: string;
  onDone?: () => void;
};

function localTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

export function SummaryEditor({
  currentPubkey,
  record,
  draftId,
  onDone,
}: SummaryViewProps) {
  const { activeCommunity } = useCommunities();
  const [name, setName] = React.useState(record?.config.name ?? "New Summary");
  const [audience, setAudience] = React.useState<SummaryAudience>(
    record?.config.audience ?? { mode: "only_me" },
  );
  const queryClient = useQueryClient();
  const timezone = React.useMemo(() => localTimezone(), []);
  const [editing, setEditing] = React.useState(true);
  const channelsQuery = useChannelsQuery();
  const agentsQuery = useManagedAgentsQuery();
  const personasQuery = usePersonasQuery();
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const createAgentMutation = useCreateManagedAgentMutation();
  const profileKeys = (channelsQuery.data ?? []).flatMap(
    (channel) => channel.participantPubkeys,
  );
  const profilesQuery = useUsersBatchQuery(profileKeys);
  const conversations = React.useMemo(
    () =>
      conversationsFromChannels(
        channelsQuery.data ?? [],
        currentPubkey,
        profilesQuery.data?.profiles,
      ),
    [channelsQuery.data, currentPubkey, profilesQuery.data],
  );
  const nectarAgent = React.useMemo(
    () => findNectarAgent(agentsQuery.data ?? []),
    [agentsQuery.data],
  );

  const configQuery = useQuery({
    queryKey: [
      "summary-editor",
      activeCommunity?.relayUrl,
      currentPubkey,
      record?.event.id,
      draftId,
    ],
    enabled: Boolean(currentPubkey),
    queryFn: async () => record?.event ?? null,
  });
  const payload = record
    ? {
        ...record.config,
        hour: 9,
        minute: 0,
        schedule: record.config.schedule
          ? { on: "schedule" as const, ...record.config.schedule }
          : null,
      }
    : null;
  const hasSavedConfig = Boolean(payload);
  const showBuilder = !hasSavedConfig || editing;

  const [sourceConfig, setSourceConfig] = React.useState<SummarySourceConfig>(
    () => defaultSourceConfig(conversations),
  );
  const [schedule, setSchedule] = React.useState<SummarySchedule | null>({
    on: "schedule",
    cron: "0 9 * * *",
  });
  const [reporterMode, setReporterMode] =
    React.useState<SummaryReporterMode>("nectar");
  const [reporterPubkey, setReporterPubkey] = React.useState<string | null>(
    null,
  );
  const [runtimeDraft, setRuntimeDraft] =
    React.useState<SummaryRuntimeDraft | null>(null);
  const initialized = React.useRef(false);
  React.useEffect(() => {
    if (
      initialized.current ||
      !configQuery.isFetched ||
      !channelsQuery.isFetched ||
      !agentsQuery.isFetched
    )
      return;
    initialized.current = true;
    if (!payload) {
      setSourceConfig(defaultSourceConfig(conversations));
      setSchedule({ on: "schedule", cron: "0 9 * * *" });
      setReporterMode("nectar");
      setReporterPubkey(nectarAgent?.pubkey ?? null);
      return;
    }
    setSourceConfig({
      autoInclude: payload.auto_include,
      include: [...payload.channels, ...payload.dms],
      exclude: payload.exclude,
    });
    setSchedule(
      payload.schedule === undefined
        ? { on: "schedule", cron: `${payload.minute} ${payload.hour} * * *` }
        : payload.schedule,
    );
    setReporterPubkey(payload.reporter_pubkey);
    const assignedNectar =
      Boolean(nectarAgent) &&
      payload.reporter_pubkey?.toLowerCase() ===
        nectarAgent?.pubkey.toLowerCase();
    setReporterMode(
      assignedNectar || !payload.reporter_pubkey ? "nectar" : "existing",
    );
  }, [
    conversations,
    nectarAgent,
    payload,
    configQuery.isFetched,
    channelsQuery.isFetched,
    agentsQuery.isFetched,
  ]);

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
          if (!runtimeDraft) {
            throw new Error("Choose a harness for Nectar.");
          }
          reporter = await ensureNectarReporter({
            agents: agentsQuery.data ?? [],
            personas: personasQuery.data ?? [],
            runtime: runtimeDraft.runtime,
            model: runtimeDraft.model,
            provider: runtimeDraft.provider,
            envVars: runtimeDraft.envVars,
            avatarDataUrl: await readNectarAvatarDataUrl(),
            createPersona: (input) => createPersonaMutation.mutateAsync(input),
            updatePersona: (input) => updatePersonaMutation.mutateAsync(input),
            createManagedAgent: (input) =>
              createAgentMutation.mutateAsync(input),
          });
        }
      }
      if (!reporter) {
        throw new Error("Choose Nectar's harness, or pick an existing agent.");
      }
      const reporterAgent = (agentsQuery.data ?? []).find(
        (agent) => agent.pubkey.toLowerCase() === reporter.toLowerCase(),
      );
      if (reporterMode === "existing" && !reporterAgent) {
        throw new Error(
          "The selected agent is no longer available. Choose another agent.",
        );
      }
      const lists = listsFromChecked(conversations, sourceConfig);
      if (!activeCommunity?.relayUrl) throw new Error("Choose a community.");
      if (audience.mode !== "only_me")
        for (const id of [...lists.channels, ...lists.dms]) {
          const room = channelsQuery.data?.find((c) => c.id === id);
          if (!room) throw new Error("A selected conversation is unavailable.");
          if (room.channelType === "dm" || room.visibility !== "open") {
            if (audience.mode === "community")
              throw new Error(
                `${conversations.find((c) => c.id === id)?.name ?? "A private conversation"} cannot be shared with the whole community.`,
              );
            const members = await getChannelMembers(id);
            if (
              audience.people.some(
                (person) => !members.some((m) => m.pubkey === person),
              )
            )
              throw new Error(
                `Every recipient must have access to ${conversations.find((c) => c.id === id)?.name ?? "the selected conversation"}. Change the audience or deselect it.`,
              );
          }
        }
      if (audience.mode === "selected" && !audience.people.length)
        throw new Error("Choose at least one recipient.");
      const config: NamedSummary = {
        v: 2,
        summary_id: record?.config.summary_id ?? draftId ?? crypto.randomUUID(),
        name: name.trim(),
        reporter_pubkey: reporter,
        audience,
        read_delegation: true,
        ...lists,
        auto_include: sourceConfig.autoInclude,
        schedule: schedule
          ? {
              cron: schedule.cron,
              interval: schedule.interval,
              dailyTimes: schedule.dailyTimes,
            }
          : null,
        timezone: payload?.timezone || timezone,
        paused: record?.config.paused ?? false,
        archived: false,
      };
      await saveNamedSummary(
        { owner: currentPubkey, relayUrl: activeCommunity.relayUrl },
        config,
        record,
      );
    },
    onSuccess: async () => {
      setEditing(false);
      onDone?.();
      await queryClient.invalidateQueries({ queryKey: ["brief-config"] });
      await queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
    },
  });

  if (
    configQuery.isError ||
    channelsQuery.isError ||
    agentsQuery.isError ||
    personasQuery.isError
  ) {
    return (
      <div role="alert" className="space-y-3">
        <p>
          Could not load Summary setup. Your saved settings have not been
          changed.
        </p>
        <Button
          onClick={() => {
            void configQuery.refetch();
            void channelsQuery.refetch();
            void agentsQuery.refetch();
            void personasQuery.refetch();
          }}
        >
          Retry
        </Button>
      </div>
    );
  }
  if (
    configQuery.isLoading ||
    channelsQuery.isLoading ||
    agentsQuery.isLoading ||
    personasQuery.isLoading
  ) {
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
        identityFields={
          <label htmlFor="summary-name" className="block space-y-2">
            <span>Summary name</span>
            <Input
              id="summary-name"
              aria-label="Summary name"
              value={name}
              maxLength={200}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
        }
        audienceFields={
          <SummaryAudienceFields
            scope={{
              owner: currentPubkey ?? "",
              relayUrl: activeCommunity?.relayUrl ?? "",
            }}
            value={audience}
            onChange={setAudience}
          />
        }
        detailsValid={Boolean(name.trim())}
        agents={agentsQuery.data ?? []}
        conversations={conversations}
        error={
          saveMutation.error instanceof Error
            ? saveMutation.error.message
            : saveMutation.error
              ? "Failed to save Summary sources"
              : null
        }
        schedule={schedule}
        nectarExists={Boolean(nectarAgent)}
        onCancel={
          onDone ??
          (hasSavedConfig
            ? () => {
                initialized.current = false;
                setEditing(false);
              }
            : undefined)
        }
        onRuntimeDraftChange={setRuntimeDraft}
        runtimeValid={runtimeDraft !== null}
        onReporterChange={setReporterPubkey}
        onReporterModeChange={setReporterMode}
        onSave={() => saveMutation.mutate()}
        onSourceConfigChange={setSourceConfig}
        onScheduleChange={setSchedule}
        reporterMode={reporterMode}
        reporterPubkey={reporterPubkey}
        saving={saveMutation.isPending}
        sourceConfig={sourceConfig}
        timezone={payload?.timezone || timezone}
      />
    );
  }

  return null;
}

export { SummariesView as SummaryView } from "./SummariesView";
