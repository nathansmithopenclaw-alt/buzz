import type { DictationSettings, SpeechModel } from "@/features/dictation/api";

declare global {
  interface Window {
    __BUZZ_E2E_DICTATION__?: { lowSpace?: boolean; saved?: DictationSettings };
  }
}

let settings: DictationSettings | undefined;
let models: SpeechModel[] | undefined;

/** Small native boundary fixture; the settings component and selectors stay real. */
export function dictationMockCommand(
  command: string,
  payload: unknown,
): { result: unknown } | null {
  const fixture = window.__BUZZ_E2E_DICTATION__;
  if (!fixture) return null;
  settings ??= fixture.saved ?? {
    version: 2,
    speech: { kind: "local", modelId: "parakeet-tdt-0.6b-v2-int8" },
    cleanup: { kind: "off" },
    timeoutMs: 4000,
    hotkey: "AltRight",
    language: "en",
    cloudCleanupEnabled: true,
  };
  models ??= [2, 3].map((version) => ({
    id: `parakeet-tdt-0.6b-v${version}-int8`,
    name: `Parakeet TDT 0.6B v${version} int8`,
    family: "nemo_transducer",
    language: version === 2 ? "English" : "25 languages",
    repository: "csukuangfj/parakeet",
    revision: "pinned",
    license: "CC-BY-4.0",
    recommended: version === 2,
    sizeBytes: version === 2 ? 661190513 : 670478772,
    status: version === 2 ? "ready" : "not_downloaded",
    requiredBytes: 1800000000,
    freeBytes: fixture.lowSpace ? 100000000 : 20000000000,
    enoughSpace: !fixture.lowSpace,
  }));
  const input = payload as { modelId?: string; settings?: DictationSettings };
  switch (command) {
    case "get_dictation_settings":
      return { result: settings };
    case "save_dictation_settings":
      settings = input.settings;
      return { result: settings };
    case "dictation_speech_catalog":
      return { result: models };
    case "dictation_model_status":
      return {
        result: models.find((model) => model.id === settings?.speech.modelId),
      };
    case "dictation_download_model": {
      const model = models.find((model) => model.id === input.modelId);
      if (model) model.status = { downloading: { progress_percent: 42 } };
      return { result: null };
    }
    case "dictation_remove_model": {
      const model = models.find((model) => model.id === input.modelId);
      if (model) model.status = "not_downloaded";
      return { result: null };
    }
    case "dictation_cleanup_sources":
      return { result: { agentOpenRouter: true } };
    case "dictation_cleanup_catalog":
      return {
        result: {
          agentOpenRouter: true,
          openRouter: [
            { id: "google/gemini-3.8-flash", name: "Gemini 3.8 Flash" },
            { id: "text-only-model", name: "Text-only model" },
          ],
          localConnectionIds: ["spark"],
          error: null,
        },
      };
    case "codex_connection_workspace_reason":
      return { result: null };
    case "list_model_connections":
      return {
        result: [
          {
            id: "remote",
            name: "My server",
            base_url: "https://models.example/v1",
            models: ["custom-text"],
            has_key: true,
            tested_model: null,
            tested_at: null,
          },
          {
            id: "spark",
            name: "Spark 1 · Qwen",
            base_url: "http://100.80.202.75:8010/v1",
            models: ["Qwen 3.8 27B"],
            has_key: false,
            tested_model: null,
            tested_at: null,
          },
        ],
      };
    case "list_subscription_accounts":
      return {
        result: [
          {
            id: "claude-ready",
            harness: "claude",
            provider: "anthropic",
            kind: "token",
            nickname: "Personal",
            state: "ready",
          },
        ],
      };
    case "discover_agent_models":
      return {
        result: {
          models: [
            { id: "claude-haiku-4-5", name: "Claude Haiku 4.5" },
            { id: "claude-sonnet-4-6", name: "Claude Sonnet 4.6" },
          ],
          supportsSwitching: true,
        },
      };
    default:
      return null;
  }
}
