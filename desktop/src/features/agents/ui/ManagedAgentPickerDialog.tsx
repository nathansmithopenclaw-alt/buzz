import * as React from "react";
import { Search } from "lucide-react";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/shared/ui/dialog";
import {
  MODAL_SEARCH_INPUT_CLASS,
  MODAL_SEARCH_SHELL_CLASS,
} from "@/shared/ui/modalSearchStyles";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/** Shared managed-agent roster and confirmation dialog for delegation and Summary. */
export function ManagedAgentPickerDialog({
  open,
  onOpenChange,
  title,
  description,
  selected,
  onSelectionChange,
  onConfirm,
  saving = false,
  multiple = true,
  error,
  testId = "agent-management-picker",
  extraAgents = [],
  confirmLabel = "Confirm",
  confirmDisabled = false,
  selectAllButton = false,
  onBack,
  children,
}: {
  confirmLabel?: string;
  confirmDisabled?: boolean;
  selectAllButton?: boolean;
  onBack?: () => void;
  children?: React.ReactNode;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  selected: string[];
  onSelectionChange: (keys: string[]) => void;
  onConfirm: () => void;
  saving?: boolean;
  multiple?: boolean;
  error?: React.ReactNode;
  testId?: string;
  extraAgents?: { pubkey: string; name: string; avatarUrl: string | null }[];
}) {
  const query = useManagedAgentsQuery();
  const [search, setSearch] = React.useState("");
  React.useEffect(() => {
    if (open) setSearch("");
  }, [open]);
  const agents = [
    ...extraAgents,
    ...(query.data ?? []).filter((a) => a.pubkey),
  ].sort((a, b) => a.name.localeCompare(b.name));
  const visible = agents.filter((a) =>
    a.name.toLowerCase().includes(search.trim().toLowerCase()),
  );
  const checked = (key: string) =>
    selected.some((value) => value.toLowerCase() === key.toLowerCase());
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!saving) onOpenChange(next);
      }}
    >
      <DialogContent className="max-w-md" data-testid={testId}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
          <label
            className={MODAL_SEARCH_SHELL_CLASS}
            htmlFor={`${testId}-search`}
          >
            <Search
              aria-hidden="true"
              className="h-4 w-4 shrink-0 text-muted-foreground/55"
            />
            <input
              aria-label="Search agents"
              autoCapitalize="none"
              autoCorrect="off"
              className={MODAL_SEARCH_INPUT_CLASS}
              data-testid={`${testId}-search`}
              id={`${testId}-search`}
              disabled={saving}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search agents"
              spellCheck={false}
              value={search}
            />
          </label>
        </DialogHeader>
        <div className="rounded-xl border border-border/70">
          {multiple ? (
            <div className="flex items-center justify-between gap-3 border-b border-border/60 px-3 py-2.5">
              {selectAllButton ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={saving}
                  onClick={() => onSelectionChange(agents.map((a) => a.pubkey))}
                >
                  Select all agents
                </Button>
              ) : (
                <label
                  className="flex-1 cursor-pointer text-sm font-medium"
                  htmlFor={`${testId}-all`}
                >
                  All current agents
                </label>
              )}
              <Checkbox
                aria-label="All current agents"
                id={`${testId}-all`}
                data-testid={`${testId}-all`}
                disabled={saving}
                checked={
                  agents.length > 0 && agents.every((a) => checked(a.pubkey))
                }
                onCheckedChange={(on) =>
                  onSelectionChange(
                    on === true ? agents.map((a) => a.pubkey) : [],
                  )
                }
              />
            </div>
          ) : null}
          <div className="max-h-[min(50vh,24rem)] overflow-y-auto">
            {visible.map((agent) => (
              <div
                className="flex items-center justify-between gap-3 px-3 py-2 hover:bg-muted/40"
                data-testid={`${testId}-agent-${agent.pubkey}`}
                key={agent.pubkey}
              >
                <label
                  className="flex min-w-0 flex-1 cursor-pointer items-center gap-2"
                  htmlFor={`${testId}-agent-${agent.pubkey}`}
                >
                  <UserAvatar
                    avatarUrl={agent.avatarUrl}
                    displayName={agent.name}
                    size="sm"
                  />
                  <span className="truncate text-sm">{agent.name}</span>
                </label>
                <Checkbox
                  id={`${testId}-agent-${agent.pubkey}`}
                  disabled={saving}
                  checked={checked(agent.pubkey)}
                  onCheckedChange={() =>
                    onSelectionChange(
                      multiple
                        ? checked(agent.pubkey)
                          ? selected.filter(
                              (k) =>
                                k.toLowerCase() !== agent.pubkey.toLowerCase(),
                            )
                          : [...selected, agent.pubkey]
                        : [agent.pubkey],
                    )
                  }
                />
              </div>
            ))}
            {!visible.length ? (
              <p className="px-3 py-3 text-sm text-muted-foreground">
                {query.isLoading
                  ? "Loading agents…"
                  : search.trim()
                    ? "No matching agents."
                    : "No managed agents yet."}
              </p>
            ) : null}
          </div>
        </div>
        {query.isError ? (
          <p role="alert" className="text-sm text-destructive">
            Could not load agents.{" "}
            <Button variant="link" onClick={() => void query.refetch()}>
              Retry
            </Button>
          </p>
        ) : null}
        {children}
        {error}
        <DialogFooter>
          {onBack ? (
            <Button
              type="button"
              variant="outline"
              disabled={saving}
              onClick={onBack}
            >
              Back
            </Button>
          ) : null}
          <Button
            disabled={saving}
            onClick={() => onOpenChange(false)}
            variant="outline"
          >
            Cancel
          </Button>
          <Button
            data-testid={`${testId}-confirm`}
            disabled={
              saving ||
              confirmDisabled ||
              query.isLoading ||
              query.isError ||
              (!multiple && selected.length !== 1)
            }
            onClick={onConfirm}
          >
            {saving ? "Saving…" : confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
