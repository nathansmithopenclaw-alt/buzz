import {
  CalendarClock,
  FileText,
  MoreHorizontal,
  UsersRound,
} from "lucide-react";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import type { SummaryDefinitionRecord } from "../lib/namedSummaries";
import { summaryScheduleLabel } from "../lib/summaryScheduleLabel";

export function SummaryCollectionCard({
  record,
  agentName,
  reportCount,
  latestAt,
  busy,
  onOpen,
  onEdit,
  onDuplicate,
  onPause,
  onArchive,
}: {
  record: SummaryDefinitionRecord;
  agentName: string;
  reportCount: number | null | undefined;
  latestAt?: number;
  busy: boolean;
  onOpen: () => void;
  onEdit: () => void;
  onDuplicate: () => void;
  onPause: () => void;
  onArchive: () => void;
}) {
  const c = record.config;
  const status = c.archived
    ? "Archived"
    : c.paused
      ? "Paused"
      : c.schedule
        ? "Scheduled"
        : "Manual";
  return (
    <article
      aria-label={c.name}
      className="flex min-w-0 flex-col rounded-xl border bg-background p-5"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <button
            type="button"
            onClick={onOpen}
            className="break-words text-left font-semibold hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          >
            {c.name}
          </button>
          <p className="mt-1 text-sm text-muted-foreground">{agentName}</p>
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label={`Actions for ${c.name}`}
            >
              <MoreHorizontal aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onEdit}>Edit</DropdownMenuItem>
            <DropdownMenuItem onSelect={onDuplicate}>
              Duplicate
            </DropdownMenuItem>
            {!c.archived ? (
              <DropdownMenuItem disabled={busy} onSelect={onPause}>
                {c.paused ? "Resume" : "Pause"}
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuSeparator />
            <DropdownMenuItem disabled={busy} onSelect={onArchive}>
              {c.archived ? "Restore" : "Archive"}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <div className="my-4 space-y-2 text-sm text-muted-foreground">
        <p className="flex items-start gap-2">
          <CalendarClock
            aria-hidden="true"
            className="mt-0.5 size-4 shrink-0"
          />
          <span>{summaryScheduleLabel(c)}</span>
        </p>
        <p className="flex items-center gap-2">
          <UsersRound aria-hidden="true" className="size-4 shrink-0" />
          <span>
            {c.audience.mode === "only_me"
              ? "Only me"
              : c.audience.mode === "community"
                ? "Everyone in this community"
                : `${c.audience.people.length} selected people`}
          </span>
        </p>
        <p className="flex items-center gap-2">
          <FileText aria-hidden="true" className="size-4 shrink-0" />
          <span>
            {c.channels.length + c.dms.length} conversations
            {typeof reportCount === "number"
              ? ` · ${reportCount} ${reportCount === 1 ? "recent report" : "recent reports"}`
              : ""}
          </span>
        </p>
      </div>
      <div className="mt-auto flex flex-wrap items-center justify-between gap-3 border-t pt-3">
        <div className="space-y-1">
          <span className="rounded-md bg-muted px-2 py-1 text-xs font-medium">
            {status}
          </span>
          <p className="pt-1 text-xs text-muted-foreground">
            {reportCount === null
              ? "History unavailable"
              : reportCount === undefined
                ? "Loading history…"
                : latestAt
                  ? `Latest ${new Date(latestAt * 1000).toLocaleDateString()}`
                  : "No reports yet"}
          </p>
        </div>
        <Button size="sm" variant="outline" onClick={onOpen}>
          Open
        </Button>
      </div>
    </article>
  );
}
