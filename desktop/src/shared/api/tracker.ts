import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";

export function readTrackerMembership(input: {
  workspaceId: string;
  managedSigner: string;
}): Promise<RelayEvent> {
  return invokeTauri<RelayEvent>("read_tracker_membership", input);
}

type TrackerFoldInput = {
  events: RelayEvent[];
  taskEvents: RelayEvent[];
  workspaceId: string;
  managedSigner: string;
  snapshotAt: number;
};

export function foldTrackerContext<T>(
  input: TrackerFoldInput & {
    membershipEvent: RelayEvent;
    advanceCursor: boolean;
  },
): Promise<T> {
  return invokeTauri<T>("fold_tracker_context", input);
}

export function foldTrackerContextIncremental<T>(
  input: TrackerFoldInput,
): Promise<T> {
  return invokeTauri<T>("fold_tracker_context_incremental", input);
}

export function trackerFoldCursor(input: {
  workspaceId: string;
  managedSigner: string;
}): Promise<number | null> {
  return invokeTauri<number | null>("tracker_fold_cursor", input);
}

export function resumeTrackerContext<T>(
  input: TrackerFoldInput & {
    membershipEvent: RelayEvent;
    advanceCursor: boolean;
  },
): Promise<T> {
  return invokeTauri<T>("resume_tracker_context", input);
}

/** Reconstruct a read-only local preview after current workspace membership is checked. */
export function readTrackerContextCache<T>(input: {
  workspaceId: string;
  managedSigner: string;
  membershipEvent: RelayEvent;
}): Promise<T | null> {
  return invokeTauri<T | null>("read_tracker_context_cache", input);
}
