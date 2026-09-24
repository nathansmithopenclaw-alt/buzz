import { invokeTauri } from "@/shared/api/tauri";

/** Where cleanup runs; mirrors `CleanupSource` in the Tauri backend. */
export type DictationCleanupSource =
  | { kind: "off" }
  | { kind: "connection"; connectionId: string; model: string }
  | { kind: "claude_plan"; accountId: string | null; model: string };

/** Persisted dictation settings; mirrors `DictationSettings`. */
export type DictationSettings = {
  version: number;
  cleanup: DictationCleanupSource;
  timeoutMs: number;
  /** `KeyboardEvent.code` for push-to-talk / tap-to-toggle. */
  hotkey: string;
  language: string;
  cloudCleanupEnabled: boolean;
};

export type DictationModelStatus =
  | "not_downloaded"
  | "ready"
  | { downloading: { progress_percent: number } }
  | { error: string };

export type DictationModelInfo = {
  status: DictationModelStatus;
  requiredBytes: number;
  freeBytes: number | null;
  enoughSpace: boolean;
};

export type DictationTranscript = {
  text: string;
  audioMs: number;
  decodeMs: number;
  tailDecodeMs: number;
  chunks: number;
};

/** Why cleanup produced no text; mirrors `CleanupFailure`. */
export type DictationCleanupFailure =
  | { kind: "not_configured" }
  | { kind: "secret_detected"; secret_kind: string }
  | { kind: "timeout" }
  | { kind: "offline" }
  | { kind: "rate_limited" }
  | { kind: "http"; status: number }
  | { kind: "bad_response" }
  | { kind: "unavailable"; message: string };

export type DictationCleanupResult = {
  text: string | null;
  failure: DictationCleanupFailure | null;
  elapsedMs: number;
};

export const getDictationModelInfo = () =>
  invokeTauri<DictationModelInfo>("dictation_model_status");

export const downloadDictationModel = () =>
  invokeTauri<void>("dictation_download_model");

export const getDictationSettings = () =>
  invokeTauri<DictationSettings>("get_dictation_settings");

export const saveDictationSettings = (settings: DictationSettings) =>
  invokeTauri<DictationSettings>("save_dictation_settings", { settings });

export const startDictation = (context: {
  roster: string[];
  vocabulary: string[];
}) =>
  invokeTauri<{ sessionId: number }>("dictation_start", { context });

export const stopDictation = (sessionId: number) =>
  invokeTauri<DictationTranscript>("dictation_stop", { sessionId });

export const cancelDictation = (sessionId: number) =>
  invokeTauri<void>("dictation_cancel", { sessionId });

export const cleanupDictation = (request: {
  sessionId: number;
  transcript: string;
  roster: string[];
  vocabulary: string[];
}) => invokeTauri<DictationCleanupResult>("dictation_cleanup", { request });

/**
 * Sends one raw batch of 48 kHz f32 samples. Uses Tauri's internal raw-body
 * IPC (the typed API cannot send `InvokeBody::Raw`), as the huddle does.
 */
export function pushDictationAudio(bytes: Uint8Array): Promise<unknown> {
  // biome-ignore lint/suspicious/noExplicitAny: Tauri internals have no public type definition
  const internals = (window as any).__TAURI_INTERNALS__;
  if (!internals?.invoke) {
    return Promise.reject(new Error("Tauri internals not available"));
  }
  return internals.invoke("dictation_push_audio", bytes);
}

/** Plain-language reason for a fallback, shown in the dictation note. */
export function describeCleanupFailure(
  failure: DictationCleanupFailure,
): string | null {
  switch (failure.kind) {
    case "not_configured":
      return null;
    case "secret_detected":
      return `Cleanup skipped: your words looked like a ${failure.secret_kind}, so nothing was sent.`;
    case "timeout":
      return "Cleanup took too long, so here are your words as spoken.";
    case "offline":
      return "Cleanup is unreachable (offline?), so here are your words as spoken.";
    case "rate_limited":
      return "Cleanup is busy right now, so here are your words as spoken.";
    case "http":
      return `Cleanup failed (HTTP ${failure.status}), so here are your words as spoken.`;
    case "bad_response":
      return "Cleanup returned nothing usable, so here are your words as spoken.";
    case "unavailable":
      return `Cleanup unavailable: ${failure.message}`;
  }
}
