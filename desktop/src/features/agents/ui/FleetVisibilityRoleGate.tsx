import * as React from "react";

import { classifyFleetVisibilityBootstrapRead } from "@/features/agents/lib/fleetVisibility";
import {
  updateFleetVisibilityRole,
  useFleetVisibilitySettings,
} from "@/features/agents/lib/useFleetVisibilitySettings";
import { relayClient } from "@/shared/api/relayClient";
import type { FleetVisibilityRole } from "@/shared/api/tauriManagedAgents";
import {
  KIND_HOST_DECLARATION,
  KIND_MANAGED_AGENT,
} from "@/shared/constants/kinds";
import { useFeatureEnabled } from "@/shared/features";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { cn } from "@/shared/lib/cn";
import { listKeyedManagedAgentPubkeys } from "@/shared/api/tauriManagedAgents";

type Props = {
  ownerPubkey?: string;
  relayUrl?: string;
};

type ChoiceState = {
  hostName: string | null;
  unmatchedCount: number;
  fleetRecordCount: number;
  detectionFailed?: boolean;
};

export function FleetVisibilityRoleGate({ ownerPubkey, relayUrl }: Props) {
  const enabled = useFeatureEnabled("fleetVisibility");
  const settings = useFleetVisibilitySettings();
  const [choice, setChoice] = React.useState<ChoiceState | null>(null);
  const [selectedRole, setSelectedRole] =
    React.useState<FleetVisibilityRole>("observer");
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (
      !enabled ||
      !settings?.enabled ||
      !settings.needsChoice ||
      !ownerPubkey ||
      !relayUrl
    ) {
      setChoice(null);
      return;
    }
    let cancelled = false;

    const showChoice = (
      agents: readonly string[],
      events: Parameters<typeof classifyFleetVisibilityBootstrapRead>[2],
      startedConnectionState: ReturnType<typeof relayClient.getConnectionState>,
    ) => {
      if (cancelled) return false;
      const decision = classifyFleetVisibilityBootstrapRead(
        startedConnectionState,
        relayClient.getConnectionState(),
        events,
        agents,
        settings.hostId,
      );
      if (decision.kind === "auto-host") return true;
      setSelectedRole("observer");
      setChoice({
        hostName: decision.bootstrap.hostName,
        unmatchedCount: decision.bootstrap.unmatchedAgentPubkeys.length,
        fleetRecordCount: decision.fleetRecordCount,
        detectionFailed: decision.detectionFailed,
      });
      return false;
    };

    const startedConnectionState = relayClient.getConnectionState();
    if (startedConnectionState !== "connected") {
      showChoice([], null, startedConnectionState);
      return () => {
        cancelled = true;
      };
    }

    void Promise.all([
      listKeyedManagedAgentPubkeys(),
      relayClient.fetchEvents({
        authors: [ownerPubkey],
        kinds: [KIND_MANAGED_AGENT, KIND_HOST_DECLARATION],
        limit: 500,
      }),
    ])
      .then(async ([agents, events]) => {
        if (cancelled) return;
        if (showChoice(agents, events, startedConnectionState)) {
          await updateFleetVisibilityRole("host");
        }
      })
      .catch((bootstrapError) => {
        console.warn(
          "[FleetVisibility] first-run detection failed:",
          bootstrapError,
        );
        showChoice([], null, startedConnectionState);
      });
    return () => {
      cancelled = true;
    };
  }, [
    enabled,
    ownerPubkey,
    relayUrl,
    settings?.enabled,
    settings?.hostId,
    settings?.needsChoice,
  ]);

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      await updateFleetVisibilityRole(selectedRole);
      setChoice(null);
    } catch (saveError) {
      setError(
        saveError instanceof Error
          ? saveError.message
          : "Could not save this device role.",
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={choice !== null}>
      <DialogContent
        className="max-w-lg"
        onEscapeKeyDown={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
        showCloseButton={false}
      >
        <DialogHeader>
          <DialogTitle>How should this device use your agents?</DialogTitle>
          <DialogDescription>
            {choice?.detectionFailed ? (
              <>Choose a role to finish enabling Fleet Visibility.</>
            ) : (choice?.unmatchedCount ?? 0) === 0 ? (
              <>
                We found {choice?.fleetRecordCount ?? 0} existing fleet record
                {(choice?.fleetRecordCount ?? 0) === 1 ? "" : "s"} for this
                account. Choose whether this device should host agents or only
                observe them.
              </>
            ) : (
              <>
                We found {choice?.unmatchedCount ?? 0} agent
                {(choice?.unmatchedCount ?? 0) === 1 ? "" : "s"} hosted
                elsewhere
                {choice?.hostName ? ` on ${choice.hostName}` : ""}. An Observer
                can see and message them without starting duplicate agents here.
              </>
            )}
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3 sm:grid-cols-2">
          {(
            [
              {
                role: "observer" as const,
                title: "Join as Observer",
                description:
                  "See and message the fleet. Never run or change agents here.",
              },
              {
                role: "host" as const,
                title: "Host agents here",
                description:
                  "This device may create, run, stop, and configure agents.",
              },
            ] as const
          ).map((option) => (
            <button
              className={cn(
                "rounded-xl border p-4 text-left transition-colors",
                selectedRole === option.role
                  ? "border-primary bg-primary/8"
                  : "border-border/70 hover:bg-muted/50",
              )}
              key={option.role}
              onClick={() => setSelectedRole(option.role)}
              type="button"
            >
              <span className="block text-sm font-medium">{option.title}</span>
              <span className="mt-1 block text-xs text-muted-foreground">
                {option.description}
              </span>
            </button>
          ))}
        </div>
        {error ? <p className="text-sm text-destructive">{error}</p> : null}
        <DialogFooter>
          <Button disabled={saving} onClick={() => void save()}>
            {saving ? "Saving…" : "Continue"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
