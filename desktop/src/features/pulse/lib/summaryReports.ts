import type { RelayEvent } from "@/shared/api/types";
import { summaryOperation, type SummaryScope } from "./namedSummaries";
export type SummaryReportItem = {
  id: string;
  pubkey: string;
  created_at: number;
  title: string;
  owner: string;
  summary_id: string;
  summary_name: string;
  agent_name: string;
  owner_name: string;
  can_delete: boolean;
  since?: number;
  until?: number;
};
export type ReportCursor = { before_time: number; before_id: string };
export type SummaryReportPage = {
  reports: SummaryReportItem[];
  next: ReportCursor | null;
};
export function fetchSummaryReportPage(
  scope: SummaryScope,
  filter: Record<string, unknown>,
) {
  return summaryOperation<SummaryReportPage>(scope, {
    summary_operation: "reports_list",
    metadata_only: true,
    filter,
  });
}
export async function fetchSummaryReport(scope: SummaryScope, id: string) {
  const result = await summaryOperation<{ event: RelayEvent }>(scope, {
    summary_operation: "reports_get",
    event_ids: [id],
  });
  return result.event;
}
export type SummaryOverview = {
  next_run_at?: string | null;
  summary_id: string;
  report_count: number;
  latest_at: number | null;
  state: string | null;
  error: string | null;
  last_run_at: string | null;
};
