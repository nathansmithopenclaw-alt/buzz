import { getIdentity } from "@/shared/api/tauriIdentity";
import { getRelayWsUrl, invokeTauri } from "@/shared/api/tauri";
import { dispatchHistoryClient } from "./relayHistory";

/** Fresh relay membership proves a copy cannot widen the source audience. */
export async function assertDispatchAudience(
  sourceChannel: string,
  destinationChannel: string,
  requiredReaders: string[] = [],
) {
  const [identity, relay] = await Promise.all([getIdentity(), getRelayWsUrl()]);
  const history = dispatchHistoryClient();
  const [sources, destinations] = await Promise.all([
    history.fetch({ kinds: [39002], "#d": [sourceChannel], limit: 1 }),
    history.fetch({ kinds: [39002], "#d": [destinationChannel], limit: 1 }),
  ]);
  if (!sources[0] || !destinations[0])
    throw new Error(
      "Current conversation permissions could not be confirmed. Share a link without copying the source text.",
    );
  await invokeTauri("validate_dispatch_audience", {
    source: sources[0],
    destination: destinations[0],
    sourceChannel,
    destinationChannel,
    viewer: identity.pubkey,
    requiredReaders,
  });
  const [stillIdentity, stillRelay] = await Promise.all([
    getIdentity(),
    getRelayWsUrl(),
  ]);
  if (stillRelay !== relay || stillIdentity.pubkey !== identity.pubkey)
    throw new Error(
      "Community or identity changed. Review the destination again.",
    );
  return { relay, signer: identity.pubkey };
}
