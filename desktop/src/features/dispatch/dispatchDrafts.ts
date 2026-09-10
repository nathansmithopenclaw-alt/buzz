import { getIdentity } from "@/shared/api/tauriIdentity";
import { getRelayWsUrl } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { setJournalItem as setStorageItem } from "./journalStorage";
import { getJournalItem as getStorageItem } from "./journalStorage";
import { withLocalJournalLock } from "./localJournalLock";

export type DraftScope = { relay: string; signer: string; workspace: string };
export type DispatchDraft = DraftScope & {
  event: RelayEvent;
  category: string;
  issueId?: string;
  savedAt: number;
};
const changed = "buzz-dispatch-drafts-changed";
const key = (value: DraftScope) =>
  `buzz-dispatch-drafts-v1:${encodeURIComponent(value.relay)}:${value.signer}:${value.workspace}`;
export async function assertDraftScope(
  value: Pick<DraftScope, "relay" | "signer">,
) {
  const [relay, identity] = await Promise.all([getRelayWsUrl(), getIdentity()]);
  if (relay !== value.relay || identity.pubkey !== value.signer)
    throw new Error(
      "Return to the original community and identity to manage this saved change.",
    );
}
function read(storageKey: string): DispatchDraft[] {
  const raw = getStorageItem(storageKey);
  if (!raw) return [];
  const value: unknown = JSON.parse(raw);
  if (
    !Array.isArray(value) ||
    value.length > 100 ||
    value.some(
      (value) =>
        !value ||
        typeof value.category !== "string" ||
        !value.event?.id ||
        value.event.pubkey !== value.signer ||
        typeof value.workspace !== "string",
    )
  )
    throw new Error(
      "Saved drafts could not be read. Their local copies have been preserved.",
    );
  return value as DispatchDraft[];
}
function write(storageKey: string, values: DispatchDraft[]) {
  const encoded = JSON.stringify(values);
  if (
    values.length > 100 ||
    new TextEncoder().encode(encoded).length > 4 * 1024 * 1024 ||
    !setStorageItem(storageKey, encoded)
  )
    throw new Error(
      "The draft could not be saved. Export or remove an older draft to free space. The original retry copy is retained.",
    );
  if (typeof window !== "undefined") window.dispatchEvent(new Event(changed));
}
/** Call while holding the originating journal lock. Archive first; only then remove its retry entry. */
export async function keepDispatchDraft(
  value: Omit<DispatchDraft, "savedAt">,
  removePending: () => void,
) {
  await assertDraftScope(value);
  await withLocalJournalLock(key(value), async () => {
    const values = read(key(value));
    if (!values.some((draft) => draft.event.id === value.event.id))
      write(key(value), [...values, { ...value, savedAt: Date.now() }]);
    await assertDraftScope(value);
    removePending();
  });
}
export async function dispatchDrafts(workspace: string) {
  const [relay, identity] = await Promise.all([getRelayWsUrl(), getIdentity()]);
  return read(key({ relay, signer: identity.pubkey, workspace }));
}
export async function removeDispatchDraft(value: DispatchDraft) {
  await assertDraftScope(value);
  await withLocalJournalLock(key(value), async () => {
    await assertDraftScope(value);
    write(
      key(value),
      read(key(value)).filter((draft) => draft.event.id !== value.event.id),
    );
  });
}
export function watchDispatchDrafts(callback: () => void) {
  window.addEventListener(changed, callback);
  window.addEventListener("storage", callback);
  return () => {
    window.removeEventListener(changed, callback);
    window.removeEventListener("storage", callback);
  };
}
/** Human-readable copy; the exact signed bytes are retained separately for export. */
export function dispatchDraftText(event: RelayEvent) {
  let value: unknown;
  try {
    value = JSON.parse(event.content);
  } catch {
    return event.content;
  }
  const lines: string[] = [];
  const hidden = new Set([
    "v",
    "request_id",
    "parent_event",
    "revision",
    "record_id",
  ]);
  const describe = (value: unknown, label = "") => {
    if (value === null || value === undefined) {
      if (label) lines.push(`${label}: clear`);
      return;
    }
    if (typeof value !== "object") {
      lines.push(label ? `${label}: ${String(value)}` : String(value));
      return;
    }
    if (Array.isArray(value)) {
      for (const item of value) describe(item, label);
      return;
    }
    for (const [name, item] of Object.entries(value))
      if (!hidden.has(name))
        describe(
          item,
          [label, name.replaceAll("_", " ")].filter(Boolean).join(" / "),
        );
  };
  describe(value);
  return lines.join("\n");
}
