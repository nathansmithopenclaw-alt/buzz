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
  }>(
    { owner, relayUrl },
    { summary_operation: "start", summary_id: config.summary_id },
  );
  if (!receipt.created)
    return {
      channelId: channel.id,
      eventId: null,
      requestedAt: Date.now(),
      run: receipt.run,
    };
  const text = `Create the Summary for owner ${owner}, summary ${config.summary_id}, run ${receipt.run.run_id}. Use buzz briefs run status --owner ${owner} --summary ${config.summary_id} --run ${receipt.run.run_id} for its definition and cursors. Read every conversation in config.channels and config.dms, including those without a cursor. Cursors are previous boundaries, not the source list; the relay supplies an initial lookback when no cursor exists. Use buzz briefs read --owner ${owner} --summary ${config.summary_id} --run ${receipt.run.run_id} --channel <id>. Publish composition JSON with buzz briefs run publish for this run. If blocked or approval is needed, record it with buzz briefs run progress using this owner, summary and run plus --state failed or --state needs_approval and --message. Resume with --state running after approval. Do not use generic report publishing or advance watermarks separately. Do not invent activity.`;
  const sent = await sendChannelMessage(
    channel.id,
    text,
    null,
    undefined,
    [reporter.pubkey],
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    relayUrl,
    owner,
  );
  return {
    channelId: channel.id,
    eventId: sent.eventId,
    requestedAt: Date.now(),
    run: receipt.run,
  };
}
