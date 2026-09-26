import { invokeTauri } from "@/shared/api/tauri";

/** Where cleanup runs; mirrors `CleanupSource` in the Tauri backend. */
export type DictationCleanupSource =
  | { kind: "off" }
  | { kind: "connection"; connectionId: string; model: string }
  /** OpenRouter using the key already in global Agent settings. */
  | { kind: "agent_open_router"; model: string }
  | { kind: "claude_plan"; accountId: string | null; model: string };

/** Persisted dictation settings; mirrors `DictationSettings`. */
export type DictationSettings = {
  version: number;
  speech: { kind: "local"; modelId: string };
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
  /** Set when captures are on and this dictation was written locally. */
  captureId?: string;
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

export const downloadDictationModel = (modelId?: string) =>
  invokeTauri<void>("dictation_download_model", { modelId });

export type SpeechModel = DictationModelInfo & {
  id: string;
  name: string;
  family: string;
  language: string;
  repository: string;
  revision: string;
  license: string;
  recommended: boolean;
  sizeBytes: number;
};
export const listSpeechModels = () =>
  invokeTauri<SpeechModel[]>("dictation_speech_catalog");
export const removeSpeechModel = (modelId: string) =>
  invokeTauri<void>("dictation_remove_model", { modelId });
export type CleanupCatalog = {
  agentOpenRouter: boolean;
  openRouter: { id: string; name: string }[];
  localConnectionIds: string[];
  error: string | null;
};
export const getCleanupCatalog = () =>
  invokeTauri<CleanupCatalog>("dictation_cleanup_catalog");

/** Keyless cleanup sources this install can use (presence only, no keys). */
export type DictationCleanupSources = { agentOpenRouter: boolean };

export const getDictationCleanupSources = () =>
  invokeTauri<DictationCleanupSources>("dictation_cleanup_sources");

export const getDictationSettings = () =>
  invokeTauri<DictationSettings>("get_dictation_settings");

export const saveDictationSettings = (settings: DictationSettings) =>
  invokeTauri<DictationSettings>("save_dictation_settings", { settings });

export const startDictation = (context: {
  roster: string[];
  vocabulary: string[];
}) => invokeTauri<{ sessionId: number }>("dictation_start", { context });

export const stopDictation = (sessionId: number) =>
  invokeTauri<DictationTranscript>("dictation_stop", { sessionId });

export const cancelDictation = (sessionId: number) =>
  invokeTauri<void>("dictation_cancel", { sessionId });

/** Polishes a dictation whose mentions are `{{n}}` placeholders. */
export const cleanupDictation = (request: {
  sessionId: number;
  transcript: string;
  vocabulary: string[];
}) => invokeTauri<DictationCleanupResult>("dictation_cleanup", { request });

/** Appends a note to a local dictation capture. */
export const noteDictationCapture = (
  captureId: string,
  note: Record<string, unknown>,
) => invokeTauri<void>("dictation_capture_note", { captureId, note });

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
