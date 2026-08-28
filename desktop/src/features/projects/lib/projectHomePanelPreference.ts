import * as React from "react";

const STORAGE_KEY = "buzz.projects.openHomePanelAutomatically";
const listeners = new Set<() => void>();

let openProjectHomePanelAutomatically = readStoredPreference();

function readStoredPreference(): boolean {
  try {
    return globalThis.localStorage?.getItem(STORAGE_KEY) !== "false";
  } catch {
    return true;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): boolean {
  return openProjectHomePanelAutomatically;
}

function getServerSnapshot(): boolean {
  return true;
}

/** Whether project home channels should open with their overview panel visible. */
export function getOpenProjectHomePanelAutomatically(): boolean {
  return openProjectHomePanelAutomatically;
}

/** Persist the device-level project home panel preference. */
export function setOpenProjectHomePanelAutomatically(enabled: boolean): void {
  if (openProjectHomePanelAutomatically === enabled) return;
  openProjectHomePanelAutomatically = enabled;

  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, String(enabled));
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }

  for (const listener of listeners) listener();
}

export function useOpenProjectHomePanelAutomatically(): boolean {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
