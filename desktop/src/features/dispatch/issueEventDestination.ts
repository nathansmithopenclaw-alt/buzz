import type { RelayEvent } from "@/shared/api/types";
import { KIND_TRACKER_ISSUE } from "@/shared/constants/kinds";

/** Resolve an immutable issue root using its signed workspace and issue tags. */
export function issueEventDestination(
  event: Pick<RelayEvent, "kind" | "tags"> | undefined,
) {
  if (event?.kind !== KIND_TRACKER_ISSUE) return null;
  const workspace = event.tags.find((tag) => tag[0] === "h")?.[1];
  const issueId = event.tags.find((tag) => tag[0] === "d")?.[1];
  if (!workspace || !issueId) return null;
  return { workspace, issueId };
}
