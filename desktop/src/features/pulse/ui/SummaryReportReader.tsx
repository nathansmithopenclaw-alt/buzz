import { useChannelsQuery } from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { conversationsFromChannels } from "../lib/summaryProtocol";
import { buildSummaryLink } from "../lib/summaryLink";
import { toast } from "sonner";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogCancel,
} from "@/shared/ui/alert-dialog";
import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import {
  fetchSummaryReport,
  type SummaryReportItem,
} from "../lib/summaryReports";
import type { SummaryScope } from "../lib/namedSummaries";
import { reportContent } from "./SummaryReportBrowser";

export function SummaryReportReader({
  scope,
  item,
  onClose,
  actions,
  onRegenerate,
}: {
  scope: SummaryScope;
  item: SummaryReportItem;
  onClose: () => void;
  actions?: React.ReactNode;
  onRegenerate?: () => Promise<void>;
}) {
  const channels = useChannelsQuery();
  const profiles = useUsersBatchQuery(
    (channels.data ?? []).flatMap((c) => c.participantPubkeys),
  );
  const conversations = conversationsFromChannels(
    channels.data ?? [],
    scope.owner,
    profiles.data?.profiles,
  );
  const [confirmReplay, setConfirmReplay] = React.useState(false);
  const [replaying, setReplaying] = React.useState(false);
  const [replayError, setReplayError] = React.useState<string | null>(null);
  const heading = React.useRef<HTMLHeadingElement>(null);
  const report = useQuery({
    queryKey: ["summary-report", scope.relayUrl, scope.owner, item.id],
    queryFn: () => fetchSummaryReport(scope, item.id),
    refetchInterval: 15000,
    gcTime: 0,
    staleTime: 0,
  });
  const content = report.isError
    ? null
    : report.data
      ? reportContent(report.data)
      : null;
  let sources: string[] = [];
  if (content && report.data) {
    try {
      const raw = JSON.parse(report.data.content).sources;
      if (Array.isArray(raw))
        sources = raw.filter((id): id is string => typeof id === "string");
    } catch {
      /* Unsupported legacy metadata does not hide a readable report. */
    }
  }
  const focusedReport = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (!report.isLoading && focusedReport.current !== item.id) {
      focusedReport.current = item.id;
      heading.current?.focus();
      heading.current?.scrollIntoView({ block: "start" });
    }
  }, [item.id, report.isLoading]);
  const exportReport = () => {
    if (!content) return;
    const url = URL.createObjectURL(
      new Blob([`# ${content.title}\n\n${content.markdown}\n`], {
        type: "text/markdown;charset=utf-8",
      }),
    );
    const link = document.createElement("a");
    link.href = url;
    link.download = `summary-${new Date(item.created_at * 1000).toISOString().slice(0, 10)}.md`;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  return (
    <article
      aria-label="Selected report"
      className="space-y-4 rounded-xl border bg-background p-5 sm:p-6"
    >
      <div className="flex flex-wrap gap-2">
        <Button variant="ghost" onClick={onClose}>
          Back to reports
        </Button>
        {actions}
      </div>
      <header className="space-y-2 border-b pb-4">
        <h3
          ref={heading}
          tabIndex={-1}
          className="text-xl font-semibold outline-none"
        >
          {item.title}
        </h3>
        <p className="text-sm text-muted-foreground">
          {item.summary_name} · {item.agent_name} ·{" "}
          {item.owner === scope.owner
            ? "Created for you"
            : `Shared by ${item.owner_name}`}
        </p>
        <p className="text-xs text-muted-foreground">
          Published {new Date(item.created_at * 1000).toLocaleString()}
        </p>
        {item.since != null && item.until != null && (
          <p className="text-xs text-muted-foreground">
            Covers {new Date(item.since * 1000).toLocaleString()} through{" "}
            {new Date(item.until * 1000).toLocaleString()}
          </p>
        )}
        {sources.length > 0 && (
          <details className="text-sm">
            <summary className="cursor-pointer">
              Conversations covered ({sources.length})
            </summary>
            <ul className="mt-2 list-disc pl-5">
              {sources.map((id) => (
                <li key={id}>
                  {conversations.find((c) => c.id === id)?.name ??
                    `Conversation ${id}`}
                </li>
              ))}
            </ul>
          </details>
        )}
      </header>
      {report.isError ? (
        <div role="alert">
          This report could not be loaded. Access may have changed.{" "}
          <Button variant="outline" onClick={() => void report.refetch()}>
            Retry
          </Button>
        </div>
      ) : report.isLoading ? (
        <p>Loading report…</p>
      ) : content ? (
        <>
          <div className="flex flex-wrap gap-2">
            <Button
              variant="outline"
              onClick={() => copyTextToClipboard(content.markdown)}
            >
              Copy report
            </Button>
            {onRegenerate ? (
              <Button variant="outline" onClick={() => setConfirmReplay(true)}>
                Regenerate this period
              </Button>
            ) : null}
            <Button
              variant="outline"
              onClick={() =>
                copyTextToClipboard(
                  buildSummaryLink(item.id, scope.relayUrl),
                  "Report link copied",
                )
              }
            >
              Copy link
            </Button>
            <Button variant="outline" onClick={exportReport}>
              Download Markdown
            </Button>
          </div>
          <Markdown content={content.markdown} />
        </>
      ) : (
        <p role="alert">This report has an unsupported format.</p>
      )}
      <AlertDialog open={confirmReplay} onOpenChange={setConfirmReplay}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Regenerate this reporting period?
            </AlertDialogTitle>
            <AlertDialogDescription>
              A new report will cover the same period as “{item.title}” using
              this Summary’s currently saved agent, conversations and audience.
              The original report stays available. Scheduled reporting progress
              will not change.
            </AlertDialogDescription>
          </AlertDialogHeader>
          {replayError ? <p role="alert">{replayError}</p> : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={replaying}>Cancel</AlertDialogCancel>
            <Button
              disabled={replaying}
              onClick={async () => {
                setReplaying(true);
                setReplayError(null);
                try {
                  await onRegenerate?.();
                  setConfirmReplay(false);
                  toast.success("Regeneration request delivered");
                } catch (error) {
                  setReplayError(
                    error instanceof Error
                      ? error.message
                      : "Regeneration could not be started",
                  );
                } finally {
                  setReplaying(false);
                }
              }}
            >
              {replaying ? "Requesting…" : "Regenerate report"}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </article>
  );
}
