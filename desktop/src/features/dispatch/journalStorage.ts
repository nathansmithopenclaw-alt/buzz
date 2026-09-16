import { setStorageItem } from "@/shared/lib/safeStorage";
/** Durable operations must fail closed when local storage cannot be read. */
export function getJournalItem(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    throw new Error(
      "Saved Dispatch changes cannot be read on this device. Restore local storage access before sending another change.",
    );
  }
}

export const DISPATCH_JOURNAL_CHANGED = "buzz-dispatch-journal-changed";
export function setJournalItem(key: string, value: string): boolean {
  const saved = setStorageItem(key, value);
  if (saved) window.dispatchEvent(new Event(DISPATCH_JOURNAL_CHANGED));
  return saved;
}
