import {
  summaryOperation,
  type SummaryDefinitionRecord,
  type SummaryRunReceipt,
} from "./namedSummaries";
import { listManagedAgents } from "@/shared/api/tauri";
import { openDm } from "@/shared/api/tauriChannels";
import { startManagedAgent } from "@/shared/api/tauriManagedAgents";
import { sendChannelMessage } from "@/shared/api/tauriMessages";
import { configFromEvent, fetchBriefConfig } from "./summaryProtocol";
import { summaryPrompt } from "./summarySchedule";

/** Run against the saved scope without changing its schedule or read grants. */
export async function runSummary(ownerPubkey: string, relayUrl: string) {
  if (!ownerPubkey || !relayUrl) throw new Error("Sign in to run a Summary.");
  const event = await fetchBriefConfig(ownerPubkey);
  const config = event ? configFromEvent(event) : null;
  if (!config?.reporter_pubkey || !config.read_delegation) {
    throw new Error("Save your Summary setup before running it.");
  }
  if (config.channels.length + config.dms.length === 0) {
    throw new Error("Choose at least one conversation in Summary settings.");
  }
  const agents = await listManagedAgents();
  const reporter = agents.find(
    (agent) =>
      agent.pubkey.toLowerCase() === config.reporter_pubkey?.toLowerCase(),
  );
  if (!reporter)
    throw new Error(
      "The Summary agent is unavailable. Choose an agent in Summary settings.",
    );
  const scope = {
    expectedRelayUrl: relayUrl,
    expectedSignerPubkey: ownerPubkey,
  };
  if (reporter.status !== "running")
    await startManagedAgent(reporter.pubkey, scope);
  const channel = await openDm({ pubkeys: [reporter.pubkey], ...scope });
  const sent = await sendChannelMessage(
    channel.id,
    summaryPrompt(ownerPubkey, reporter.name),
    null,
    undefined,
    [reporter.pubkey],
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    relayUrl,
    ownerPubkey,
  );
  return {
    channelId: channel.id,
    eventId: sent.eventId,
    requestedAt: Date.now(),
  };
}

/** Named runs are claimed before dispatch; duplicate clicks reuse the durable receipt. */
export async function runNamedSummary(
  owner: string,
  relayUrl: string,
  record: SummaryDefinitionRecord,
  sourceReport?: string,
) {
  if (record.legacy) return runSummary(owner, relayUrl);
  const config = record.config;
  if (config.archived) throw new Error("Archived Summaries cannot run.");
  const scope = { expectedRelayUrl: relayUrl, expectedSignerPubkey: owner };
  const agents = await listManagedAgents();
  const reporter = agents.find((a) => a.pubkey === config.reporter_pubkey);
  if (!reporter) throw new Error("Choose an available Summary agent.");
  if (reporter.status !== "running")
    await startManagedAgent(reporter.pubkey, scope);
  const channel = await openDm({ pubkeys: [reporter.pubkey], ...scope });
  const receipt = await summaryOperation<{
    run: SummaryRunReceipt;
    created: boolean;
    event_id?: string;
  }>(
    { owner, relayUrl },
    {
      summary_operation: "dispatch",
      summary_id: config.summary_id,
      source_report: sourceReport,
    },
  );
  return {
    channelId: channel.id,
    eventId: receipt.event_id ?? null,
    requestedAt: Date.now(),
    run: receipt.run,
    alreadyRunning: !receipt.created,
  };
}
