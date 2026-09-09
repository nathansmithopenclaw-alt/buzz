import * as React from "react";
import { FileText } from "lucide-react";
import type { RelayEvent } from "@/shared/api/types";
import { Markdown } from "@/shared/ui/markdown";
import { cn } from "@/shared/lib/cn";

export function reportContent(event: RelayEvent) {
  try {
    const value = JSON.parse(event.content);
    return typeof value.title === "string" && typeof value.markdown === "string"
      ? (value as { title: string; markdown: string })
      : null;
  } catch {
    return null;
  }
}

/** Keep history navigable without rendering every report into one long feed. */
export function SummaryReportBrowser({ events }: { events: RelayEvent[] }) {
  const [selected, setSelected] = React.useState<string | null>(null);
  const reports = events
    .filter((event) => reportContent(event))
    .sort((a, b) => b.created_at - a.created_at || b.id.localeCompare(a.id));
  const current = reports.find((event) => event.id === selected) ?? reports[0];
  if (!current) return null;
  const content = reportContent(current);
  if (!content) return null;
  return (
    <div className="grid min-w-0 gap-5 md:grid-cols-[14rem_minmax(0,1fr)]">
      <nav aria-label="Report history" className="min-w-0">
        <h3 className="mb-3 text-sm font-medium text-muted-foreground">
          Reports <span className="ml-1 tabular-nums">{reports.length}</span>
        </h3>
        <div className="max-h-64 space-y-1 overflow-y-auto md:max-h-[65vh]">
          {reports.map((event, index) => (
            <button
              key={event.id}
              type="button"
              aria-current={event.id === current.id ? "true" : undefined}
              onClick={() => setSelected(event.id)}
              className={cn(
                "flex w-full items-start gap-2 rounded-lg p-3 text-left text-sm transition-colors hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
                event.id === current.id && "bg-muted",
              )}
            >
              <FileText
                aria-hidden="true"
                className="mt-0.5 size-4 shrink-0 text-muted-foreground"
              />
              <span className="min-w-0">
                <span className="block font-medium">
                  {new Date(event.created_at * 1000).toLocaleDateString(
                    undefined,
                    { month: "short", day: "numeric", year: "numeric" },
                  )}
                  {index === 0 ? " · Latest" : ""}
                </span>
                <span className="mt-1 block line-clamp-2 text-xs text-muted-foreground">
                  {reportContent(event)?.title}
                </span>
                <span className="mt-1 block text-xs text-muted-foreground">
                  {new Date(event.created_at * 1000).toLocaleTimeString(
                    undefined,
                    { hour: "numeric", minute: "2-digit" },
                  )}
                </span>
              </span>
            </button>
          ))}
        </div>
      </nav>
      <article
        aria-label="Selected report"
        className="min-w-0 rounded-xl border bg-background p-5 sm:p-6"
      >
        <header className="mb-6 space-y-2 border-b pb-4">
          <h3 className="text-xl font-semibold tracking-tight">
            {content.title}
          </h3>
          <p className="text-xs text-muted-foreground">
            {new Date(current.created_at * 1000).toLocaleString()}
          </p>
        </header>
        <Markdown content={content.markdown} />
      </article>
    </div>
  );
}
