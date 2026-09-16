import type { RelayEvent } from "@/shared/api/types";
import type { DispatchHistoryClient } from "./history";
/** Relay admission time can be later than the source's signed creation time.
 * Fetch exact scoped sources so a warm timestamp cursor cannot omit backdated edits.
 * The native fold separately verifies receipt signer, schema and authority. */
export async function loadAdmissionSources(
  history: Pick<DispatchHistoryClient, "fetch">,
  receipts: RelayEvent[],
  workspace: string,
  knownIds: Set<string>,
): Promise<RelayEvent[]> {
  const ids = [
    ...new Set(
      receipts
        .filter(
          (event) =>
            event.kind === 40027 &&
            event.tags.some((tag) => tag[0] === "h" && tag[1] === workspace),
        )
        .flatMap((event) =>
          event.tags
            .filter(
              (tag) => tag[0] === "e" && /^[a-f0-9]{64}$/.test(tag[1] ?? ""),
            )
            .map((tag) => tag[1]),
        ),
    ),
  ].filter((id) => !knownIds.has(id));
  if (ids.length > 30000)
    throw new Error(
      "Dispatch admission history exceeds the bounded read limit.",
    );
  const found: RelayEvent[] = [];
  for (let index = 0; index < ids.length; index += 50) {
    const batch = ids.slice(index, index + 50);
    const values = await history.fetch({
      ids: batch,
      kinds: [9, 40020, 40021, 40022, 40026],
      "#h": [workspace],
      limit: 50,
    });
    found.push(
      ...values.filter(
        (event) =>
          batch.includes(event.id) &&
          event.tags.some((tag) => tag[0] === "h" && tag[1] === workspace),
      ),
    );
    if (batch.some((id) => !found.some((event) => event.id === id)))
      throw new Error(
        "An admitted Dispatch edit is unavailable. Refresh history before making further changes.",
      );
  }
  return found;
}
