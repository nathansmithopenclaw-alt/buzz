import { AlertCircle, Loader, UsersRound } from "lucide-react";
import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  DELEGATION_CAPABILITIES,
  agentIsGranted,
  grantSubjects,
  grantedPubkeys,
  noneSubject,
  removeGranteeFromSubjects,
  subjectIsOn,
  subjectSummary,
  subjectsAreEmpty,
  type DelegationCapability,
} from "@/features/agents/delegationGrant";
import { setAgentManagementDelegation } from "@/shared/api/tauriGlobalAgentConfig";
import type {
  AgentManagementDelegationGrant,
  AgentManagementDelegationSubjects,
  CapabilitySubject,
  GlobalAgentConfig,
} from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { ManagedAgentPickerDialog } from "./ManagedAgentPickerDialog";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Switch } from "@/shared/ui/switch";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type AgentManagementDelegationCardProps = {
  grant: AgentManagementDelegationGrant | null;
  grants: Record<string, AgentManagementDelegationGrant>;
  onPersisted: (config: GlobalAgentConfig) => void;
};

type PickerState = {
  capability: DelegationCapability;
  label: string;
  draft: CapabilitySubject;
};

export function AgentManagementDelegationCard({
  grant,
  grants,
  onPersisted,
}: AgentManagementDelegationCardProps) {
  const agentsQuery = useManagedAgentsQuery();
  const agents = React.useMemo(
    () =>
      (agentsQuery.data ?? [])
        .filter((agent) => agent.pubkey)
        .slice()
        .sort((a, b) => a.name.localeCompare(b.name)),
    [agentsQuery.data],
  );
  const roster = React.useMemo(
    () => agents.map((agent) => agent.pubkey.toLowerCase()),
    [agents],
  );
  const subjects = React.useMemo(
    () => grantSubjects(grant, grants),
    [grant, grants],
  );
  const activeGrantees = React.useMemo(
    () => grantedPubkeys(subjects, roster),
    [roster, subjects],
  );
  const [picker, setPicker] = React.useState<PickerState | null>(null);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [grantAllOpen, setGrantAllOpen] = React.useState(false);

  const persist = React.useCallback(
    async (next: AgentManagementDelegationSubjects) => {
      setSaving(true);
      setError(null);
      try {
        const result = await setAgentManagementDelegation(
          subjectsAreEmpty(next) ? null : next,
        );
        onPersisted(result.config);
      } catch (err) {
        setError(
          err instanceof Error
            ? err.message
            : typeof err === "string"
              ? err
              : "Couldn't save the grant.",
        );
        throw err;
      } finally {
        setSaving(false);
      }
    },
    [onPersisted],
  );

  function openPicker(capability: DelegationCapability, label: string) {
    setError(null);
    setPicker({
      capability,
      label,
      draft: subjects[capability],
    });
  }

  async function handleSwitch(
    capability: DelegationCapability,
    label: string,
    checked: boolean,
  ) {
    if (checked) {
      openPicker(capability, label);
      return;
    }
    try {
      await persist({ ...subjects, [capability]: noneSubject() });
    } catch {
      // Error is already on the card.
    }
  }

  async function confirmPicker() {
    if (!picker) return;
    try {
      await persist({ ...subjects, [picker.capability]: picker.draft });
      setPicker(null);
    } catch {
      // Keep the picker open so the user can retry.
    }
  }

  async function confirmGrantAll() {
    try {
      const allCurrent = {
        scope: "set" as const,
        grantees: [...roster],
      };
      await persist({
        agent_config: allCurrent,
        agent_creation: allCurrent,
        env_vars: allCurrent,
        global_config: allCurrent,
        teams: allCurrent,
      });
      setGrantAllOpen(false);
    } catch {
      // Error is already on the card.
    }
  }

  async function revokeGrantee(pubkey: string) {
    try {
      await persist(removeGranteeFromSubjects(subjects, pubkey, roster));
    } catch {
      // Error is already on the card.
    }
  }

  const saveError = error ? (
    <p
      className="flex items-start gap-1.5 text-sm text-destructive"
      role="alert"
    >
      <AlertCircle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
      {error}
    </p>
  ) : null;

  return (
    <div
      className="space-y-3 rounded-lg border border-border/70 px-4 py-3"
      data-testid="agent-management-delegation-card"
    >
      <div className="space-y-1">
        <h3 className="text-sm font-medium text-foreground">
          Agent management delegation
        </h3>
        <p className="text-sm text-muted-foreground">
          Grant managed colleagues narrowly scoped authority to create agents,
          update configuration, or manage teams. You choose who gets each
          permission. Clearing a permission revokes it immediately.
        </p>
      </div>

      <Button
        data-testid="agent-management-grant-all"
        disabled={saving || roster.length === 0}
        onClick={() => {
          setError(null);
          setGrantAllOpen(true);
        }}
        size="sm"
        type="button"
        variant="outline"
      >
        Grant all permissions to current agents
      </Button>

      {activeGrantees.length > 0 ? (
        <div className="space-y-2" data-testid="agent-management-grantee-list">
          {activeGrantees.map((pubkey) => {
            const agent = agents.find(
              (candidate) => candidate.pubkey.toLowerCase() === pubkey,
            );
            const permissionCount = DELEGATION_CAPABILITIES.filter(
              ([capability]) => agentIsGranted(subjects[capability], pubkey),
            ).length;
            return (
              <div
                className="flex items-center gap-3 rounded-md border border-border/60 px-3 py-2"
                data-testid={`agent-management-grantee-${pubkey}`}
                key={pubkey}
              >
                <UserAvatar
                  avatarUrl={agent?.avatarUrl ?? null}
                  displayName={agent?.name ?? truncatePubkey(pubkey)}
                  size="sm"
                />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm">
                    {agent?.name ?? truncatePubkey(pubkey)}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {permissionCount === 1
                      ? "1 permission"
                      : `${permissionCount} permissions`}
                  </p>
                </div>
                <Button
                  data-testid={`agent-management-revoke-${pubkey}`}
                  disabled={saving}
                  onClick={() => void revokeGrantee(pubkey)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Revoke
                </Button>
              </div>
            );
          })}
        </div>
      ) : null}

      {/*
        Every row carries the "Choose agents" affordance, so this grid is the
        tallest block in the Agent defaults dialog. Keep it compact and keep
        the label on one line: at 1280x900 the dialog is capped at
        max-h-[calc(100vh-2rem)], and a second label line on the widest
        capability pushes it onto that cap, which silently absorbs the
        model-field growth canary in
        global-agent-config-screenshots.spec.ts 04.
      */}
      <div className="grid gap-2 sm:grid-cols-2">
        {DELEGATION_CAPABILITIES.map(([capability, label]) => {
          const subject = subjects[capability];
          const on = subjectIsOn(subject);
          return (
            <div
              className="flex items-center justify-between gap-2 rounded-md border border-border/60 px-3 py-1.5"
              data-testid={`agent-management-capability-${capability}`}
              key={capability}
            >
              <button
                aria-label={`Choose agents for ${label.toLowerCase()}`}
                aria-haspopup="dialog"
                className="group flex min-w-0 flex-1 cursor-pointer items-center gap-2 rounded-sm text-left outline-none hover:text-primary focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:opacity-50"
                disabled={saving}
                onClick={() => openPicker(capability, label)}
                type="button"
              >
                <span className="min-w-0 flex-1">
                  <span className="block text-sm">{label}</span>
                  <span className="flex items-center gap-1 text-xs text-muted-foreground group-hover:text-primary">
                    <UsersRound
                      aria-hidden="true"
                      className="size-3.5 shrink-0"
                    />
                    <span>Choose agents</span>
                    {on ? (
                      <>
                        <span aria-hidden="true">·</span>
                        <span
                          data-testid={`agent-management-capability-${capability}-summary`}
                        >
                          {subjectSummary(subject)}
                        </span>
                      </>
                    ) : null}
                  </span>
                </span>
              </button>
              <Switch
                aria-label={label}
                checked={on}
                disabled={saving}
                onCheckedChange={(checked) => {
                  void handleSwitch(capability, label, checked);
                }}
              />
            </div>
          );
        })}
      </div>

      {!picker && !grantAllOpen ? saveError : null}

      <ManagedAgentPickerDialog
        open={picker !== null}
        onOpenChange={(open) => {
          if (!open) setPicker(null);
        }}
        title={picker?.label ?? "Choose agents"}
        description="Choose which managed agents receive this permission."
        selected={
          picker
            ? roster.filter((key) => agentIsGranted(picker.draft, key))
            : []
        }
        onSelectionChange={(grantees) => {
          if (picker)
            setPicker({ ...picker, draft: { scope: "set", grantees } });
        }}
        onConfirm={() => void confirmPicker()}
        saving={saving}
        error={saveError}
      />

      <AlertDialog
        onOpenChange={(open) => {
          if (!saving) setGrantAllOpen(open);
        }}
        open={grantAllOpen}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Grant all permissions to current agents?
            </AlertDialogTitle>
            <AlertDialogDescription>
              Every currently managed colleague will be able to create agents,
              update configuration, set environment variables, change global
              defaults, and manage teams. Future agents remain ungranted until
              you add them.
            </AlertDialogDescription>
          </AlertDialogHeader>
          {saveError}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={saving}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              data-testid="agent-management-grant-all-confirm"
              disabled={saving}
              onClick={(event) => {
                event.preventDefault();
                void confirmGrantAll();
              }}
            >
              {saving ? <Loader className="size-3.5 animate-spin" /> : null}
              Grant all
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
