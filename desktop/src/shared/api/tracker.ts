import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";

type TrackerFoldInput = {
  events: RelayEvent[];
  taskEvents: RelayEvent[];
  workspaceId: string;
  managedSigner: string;
  snapshotAt: number;
};

export function foldTrackerContext<T>(
  input: TrackerFoldInput & { membershipEvent: RelayEvent },
): Promise<T> {
  return invokeTauri<T>("fold_tracker_context", input);
}

export function foldTrackerContextIncremental<T>(
  input: TrackerFoldInput,
): Promise<T> {
  return invokeTauri<T>("fold_tracker_context_incremental", input);
}
