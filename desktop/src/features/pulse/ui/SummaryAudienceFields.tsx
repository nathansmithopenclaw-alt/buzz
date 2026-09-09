import { truncatePubkey } from "@/shared/lib/pubkey";
import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ManagedAgentPickerDialog } from "@/features/agents/ui/ManagedAgentPickerDialog";
import { Button } from "@/shared/ui/button";
import {
  summaryOperation,
  type SummaryScope,
  type SummaryAudience,
} from "../lib/namedSummaries";

export function SummaryAudienceFields({
  scope,
  value,
  onChange,
}: {
  scope: SummaryScope;
  value: SummaryAudience;
  onChange: (value: SummaryAudience) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const [draft, setDraft] = React.useState<string[]>([]);
  const roster = useQuery({
    queryKey: ["summary-people", scope.relayUrl, scope.owner],
    queryFn: () =>
      summaryOperation<{ people: string[]; complete: boolean }>(scope, {
        summary_operation: "people",
      }),
    retry: false,
  });
  const profiles = useUsersBatchQuery(roster.data?.people ?? []);
  const people = (roster.data?.people ?? [])
    .filter((key) => key !== scope.owner)
    .map((pubkey) => ({
      pubkey,
      name:
        profiles.data?.profiles[pubkey]?.displayName || truncatePubkey(pubkey),
      avatarUrl: profiles.data?.profiles[pubkey]?.avatarUrl ?? null,
    }));
  return (
    <div className="space-y-4">
      <h3 className="font-semibold">Who can see this Summary?</h3>
      <div
        role="radiogroup"
        aria-label="Summary audience"
        className="space-y-2"
      >
        {(
          [
            ["only_me", "Only me"],
            ["community", "Everyone in this community"],
            ["selected", "Selected people"],
          ] as const
        ).map(([mode, label]) => (
          <label
            key={mode}
            className="flex items-center gap-3 rounded-xl border p-3"
          >
            <input
              type="radio"
              name="summary-audience"
              value={mode}
              checked={value.mode === mode}
              disabled={
                mode !== "only_me" &&
                (roster.isError ||
                  roster.isLoading ||
                  (mode === "community" && !roster.data?.complete))
              }
              onChange={() =>
                onChange(
                  mode === "selected"
                    ? {
                        mode,
                        people: value.mode === "selected" ? value.people : [],
                      }
                    : { mode },
                )
              }
            />
            {label}
          </label>
        ))}
      </div>
      {value.mode === "selected" ? (
        <Button
          variant="outline"
          onClick={() => {
            setDraft(value.people);
            setOpen(true);
          }}
        >
          Choose people ({value.people.length})
        </Button>
      ) : null}
      <p className="text-sm text-muted-foreground">
        Recipients must already be able to read every selected conversation.
        They can view reports; only you can edit or run this Summary.
      </p>
      {roster.isError ? (
        <p role="alert">
          The community directory is unavailable.{" "}
          <Button variant="link" onClick={() => void roster.refetch()}>
            Retry
          </Button>
        </p>
      ) : null}
      {roster.data && !roster.data.complete ? (
        <p className="text-sm text-muted-foreground">
          This relay has an incomplete people directory. Community-wide sharing
          is unavailable; you can select known members.
        </p>
      ) : null}
      <p className="text-xs text-muted-foreground">
        New recipients receive future reports. Removing someone blocks further
        relay access but cannot retract copies already downloaded.
      </p>
      <ManagedAgentPickerDialog
        open={open}
        onOpenChange={setOpen}
        title="Summary recipients"
        description="Choose people in this community, including offline members."
        selected={draft}
        onSelectionChange={setDraft}
        onConfirm={() => {
          onChange({ mode: "selected", people: draft });
          setOpen(false);
        }}
        confirmDisabled={!draft.length}
        testId="summary-people-picker"
        directory={{
          people,
          loading: roster.isLoading,
          error: roster.isError,
          retry: () => void roster.refetch(),
        }}
      />
    </div>
  );
}
