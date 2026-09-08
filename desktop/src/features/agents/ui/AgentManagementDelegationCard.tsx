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
import { Checkbox } from "@/shared/ui/checkbox";
import { Switch } from "@/shared/ui/switch";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type AgentManagementDelegationCardProps = {
  grant: AgentManagementDelegationGrant | null;
  grants: Record<string, AgentManagementDelegationGrant>;
  onPersisted: (config: GlobalAgentConfig) => void;
};

import type { AgentEditScope } from "@/shared/api/agentManagementDelegation";

type PickerState = {
  step: "recipients" | "targets";
  recipients: string[];
  index: number;
  scopes: Record<string, AgentEditScope>;
  targetDraft: CapabilitySubject;
  fields: AgentEditScope["allowedFields"];
  allowSelfEdit: boolean;
  allPermissions?: boolean;
  editRecipient?: boolean;
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
    async (
      next: AgentManagementDelegationSubjects,
      editScopes?: Record<string, AgentEditScope>,
    ) => {
      setSaving(true);
      setError(null);
      try {
        const result = await setAgentManagementDelegation(
          subjectsAreEmpty(next) ? null : next,
          editScopes,
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
      step: "recipients",
      recipients: [],
      index: 0,
      scopes: {},
      targetDraft: noneSubject(),
      fields: ["displayName"],
      allowSelfEdit: false,
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

  function targetStep(
    current: PickerState,
    recipients: string[],
    index: number,
  ): PickerState {
    const pubkey = recipients[index];
    const existing =
      grants[pubkey] ??
      (grant && "granteePubkey" in grant && grant.granteePubkey === pubkey
        ? grant
        : undefined);
    const scope =
      current.scopes[pubkey] ??
      (existing?.version === 4 ? existing.agentEdit : undefined);
    return {
      ...current,
      step: "targets",
      recipients,
      index,
      targetDraft: scope
        ? { scope: "set", grantees: scope.targetPubkeys }
        : noneSubject(),
      fields: scope?.allowedFields ?? ["displayName"],
      allowSelfEdit: scope?.allowSelfEdit ?? false,
    };
  }

  async function confirmPicker() {
    if (!picker) return;
    if (picker.capability === "agent_config" && picker.step === "recipients") {
      const recipients = roster.filter(
        (key) =>
          agentIsGranted(picker.draft, key) &&
          !agentIsGranted(subjects.agent_config, key),
      );
      if (recipients.length) {
        setPicker(targetStep(picker, recipients, 0));
        return;
      }
    }
    const scopes = { ...picker.scopes };
    if (picker.step === "targets") {
      scopes[picker.recipients[picker.index]] = {
        targetPubkeys: roster.filter((key) =>
          agentIsGranted(picker.targetDraft, key),
        ),
        allowedFields: picker.fields,
        allowSelfEdit: picker.allowSelfEdit,
      };
      if (picker.index + 1 < picker.recipients.length) {
        setPicker(
          targetStep(
            { ...picker, scopes },
            picker.recipients,
            picker.index + 1,
          ),
        );
        return;
      }
    }
    try {
      const next = picker.allPermissions
        ? (Object.fromEntries(
            DELEGATION_CAPABILITIES.map(([key]) => [key, picker.draft]),
          ) as AgentManagementDelegationSubjects)
        : { ...subjects, [picker.capability]: picker.draft };
      const selectedScopes = Object.fromEntries(
        Object.entries(scopes).filter(([key]) =>
          agentIsGranted(picker.draft, key),
        ),
      );
      await persist(
        next,
        Object.keys(selectedScopes).length ? selectedScopes : undefined,
      );
      setPicker(null);
    } catch {
      // Keep the complete draft open for retry.
    }
  }

  function confirmGrantAll() {
    setGrantAllOpen(false);
    const recipients = roster.filter(
      (key) => !agentIsGranted(subjects.agent_config, key),
    );
    const current: PickerState = {
      capability: "agent_config",
      label: "Update existing agents",
      draft: { scope: "set", grantees: [...roster] },
      step: "recipients",
      recipients,
      index: 0,
      scopes: {},
      targetDraft: noneSubject(),
      fields: ["displayName"],
      allowSelfEdit: false,
      allPermissions: true,
    };
    setPicker(recipients.length ? targetStep(current, recipients, 0) : current);
  }

  const targetMode = picker?.step === "targets";
  const pickerSelection = targetMode
    ? picker.targetDraft
    : (picker?.draft ?? noneSubject());
  function setPickerSelection(selection: CapabilitySubject) {
    if (!picker) return;
    setPicker({
      ...picker,
      ...(targetMode ? { targetDraft: selection } : { draft: selection }),
    });
  }
  const recipientName = picker
    ? agents.find((a) => a.pubkey === picker.recipients[picker.index])?.name
    : undefined;

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
                {agentIsGranted(subjects.agent_config, pubkey) &&
                roster.includes(pubkey) ? (
                  <Button
                    aria-label={`Edit agent scope for ${agent?.name ?? truncatePubkey(pubkey)}`}
                    disabled={saving}
                    onClick={() =>
                      setPicker(
                        targetStep(
                          {
                            step: "targets",
                            recipients: [pubkey],
                            index: 0,
                            scopes: {},
                            targetDraft: noneSubject(),
                            fields: ["displayName"],
                            allowSelfEdit: false,
                            capability: "agent_config",
                            label: "Update existing agents",
                            draft: subjects.agent_config,
                            editRecipient: true,
                          },
                          [pubkey],
                          0,
                        ),
                      )
                    }
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    <UsersRound aria-hidden="true" className="size-4" />
                    Edit scope
                  </Button>
                ) : null}
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
        key={`${picker?.step ?? "closed"}-${picker?.index ?? 0}`}
        open={picker !== null}
        onOpenChange={(open) => {
          if (!open) setPicker(null);
        }}
        title={
          targetMode
            ? `Which agents can ${recipientName ?? "this colleague"} modify?`
            : (picker?.label ?? "Choose agents")
        }
        description={
          targetMode
            ? "Choose targets and allowed fields. All selects current agents only; future agents require a new selection."
            : "Choose which managed colleagues receive this permission."
        }
        selected={roster.filter((key) => agentIsGranted(pickerSelection, key))}
        onSelectionChange={(grantees) =>
          setPickerSelection({ scope: "set", grantees })
        }
        onConfirm={() => void confirmPicker()}
        confirmLabel={
          picker?.capability === "agent_config" &&
          ((!targetMode &&
            roster.some(
              (key) =>
                agentIsGranted(picker.draft, key) &&
                !agentIsGranted(subjects.agent_config, key),
            )) ||
            (targetMode && picker.index + 1 < picker.recipients.length))
            ? "Next"
            : "Confirm"
        }
        confirmDisabled={
          (picker?.capability === "agent_config" &&
            !roster.some((key) => agentIsGranted(pickerSelection, key))) ||
          (targetMode && picker?.fields.length === 0)
        }
        selectAllButton={targetMode}
        onBack={
          targetMode && picker && !picker.editRecipient
            ? () => {
                const scopes = {
                  ...picker.scopes,
                  [picker.recipients[picker.index]]: {
                    targetPubkeys: roster.filter((key) =>
                      agentIsGranted(picker.targetDraft, key),
                    ),
                    allowedFields: picker.fields,
                    allowSelfEdit: picker.allowSelfEdit,
                  },
                };
                const current = { ...picker, scopes };
                setPicker(
                  picker.index > 0
                    ? targetStep(current, picker.recipients, picker.index - 1)
                    : { ...current, step: "recipients" },
                );
              }
            : undefined
        }
        saving={saving}
        error={saveError}
      >
        {targetMode && picker ? (
          <fieldset className="space-y-2" disabled={saving}>
            <legend className="text-sm font-medium">Allowed fields</legend>
            {(
              [
                ["displayName", "Display name"],
                ["systemPrompt", "Instructions (changes agent behavior)"],
              ] as const
            ).map(([field, label]) => (
              <label
                key={field}
                htmlFor={`agent-management-field-${field}`}
                className="flex items-center gap-2 text-sm"
              >
                <Checkbox
                  id={`agent-management-field-${field}`}
                  checked={picker.fields.includes(field)}
                  data-testid={`agent-management-field-${field}`}
                  onCheckedChange={(checked) =>
                    setPicker({
                      ...picker,
                      fields: checked
                        ? [...picker.fields, field]
                        : picker.fields.filter((f) => f !== field),
                    })
                  }
                />
                {label}
              </label>
            ))}
            <label
              htmlFor="agent-management-self-edit"
              className="flex items-center gap-2 text-sm"
            >
              <Checkbox
                id="agent-management-self-edit"
                checked={picker.allowSelfEdit}
                data-testid="agent-management-self-edit"
                onCheckedChange={(checked) =>
                  setPicker({ ...picker, allowSelfEdit: checked === true })
                }
              />
              Allow this colleague to edit itself when selected
            </label>
          </fieldset>
        ) : null}
      </ManagedAgentPickerDialog>

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
              set environment variables, change global defaults, and manage
              teams. Future agents remain ungranted until you add them. Next,
              choose targets and allowed fields for agent edits.
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
