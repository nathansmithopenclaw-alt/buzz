import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Editor } from "@tiptap/core";
import { toast } from "sonner";

import type { MentionCandidate } from "@/features/messages/lib/mentionCandidates";
import type { DraftMentionRef } from "@/features/messages/lib/useDrafts";
import {
  loadDraftEntry,
  saveDraftEntry,
} from "@/features/messages/lib/useDrafts";
import {
  cancelDictation,
  cleanupDictation,
  describeCleanupFailure,
  type DictationSettings,
  getDictationModelInfo,
  getDictationSettings,
  startDictation,
  stopDictation,
} from "./api";
import {
  type DictationCapture,
  MicrophoneUnavailableError,
  startDictationCapture,
} from "./lib/audioCapture";
import {
  buildDictationRoster,
  type DictationRosterEntry,
  modelRosterNames,
} from "./lib/dictationRoster";
import {
  DICTATION_SUGGESTION_EVENT,
  readDictationMark,
  releaseDictationMark,
  trackDictationMark,
} from "./lib/dictationMarks";
import {
  claimRecorder,
  type DictationStatus,
  registerDictationTarget,
  releaseRecorder,
} from "./lib/dictationSession";
import {
  insertAtSelection,
  replaceRange,
  suggestionToPickerQuery,
} from "./lib/editorOps";
import {
  type DictationSegment,
  resolveModelOutput,
  resolveVerbatim,
} from "./lib/mentionResolution";
import {
  type RegisterMention,
  renderSegments,
  replaceLastOccurrence,
} from "./lib/renderSegments";
import { applySpokenFormatting } from "./lib/spokenFormatting";

/** Longest dictation; recording stops and processes at this point. */
export const DICTATION_MAX_SECONDS = 300;
/** When the "30 seconds left" warning appears. */
export const DICTATION_WARN_SECONDS = 270;
const FLASH_MS = 1800;
const MIC_PERMISSION_SETTINGS_URL =
  "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";
/** Per-viewer convenience: the chosen dictation microphone. */
export const DICTATION_DEVICE_STORAGE_KEY = "buzz-dictation-input-device";

/** The composer's mention candidates, as the @ picker sees them. */
export type DictationCandidates = {
  candidates: readonly MentionCandidate[];
  currentPubkey: string | null;
  memberPubkeys: ReadonlySet<string>;
};

export type UseComposerDictationParams = {
  enabled: boolean;
  editor: Editor | null;
  draftKey: string | null;
  channelId: string | null;
  editTargetId: string | null;
  isMain: boolean;
  disabled: boolean;
  huddleActive: boolean;
  getCandidates: () => DictationCandidates;
  registerMention: RegisterMention;
  recentMentionPubkeys?: readonly string[];
};

type Session = {
  generation: number;
  backendId: number | null;
  capture: DictationCapture | null;
  roster: DictationRosterEntry[];
  modelNames: string[];
  settings: DictationSettings | null;
  originDraftKey: string | null;
  originChannelId: string | null;
  originEditTargetId: string | null;
  stopRequested: boolean;
};

function readStoredDevice(): string | null {
  try {
    return window.localStorage.getItem(DICTATION_DEVICE_STORAGE_KEY);
  } catch {
    return null;
  }
}

class DictationSetupError extends Error {}

function showStartError(error: unknown) {
  if (error instanceof MicrophoneUnavailableError && error.denied) {
    toast.error(error.message, {
      action: {
        label: "Open Settings",
        onClick: () =>
          void openUrl(MIC_PERMISSION_SETTINGS_URL).catch(() => {}),
      },
    });
    return;
  }
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : "Dictation could not start.";
  toast.error(message);
}

/** Adds dictated text to a saved draft (its composer is no longer showing it). */
function deliverToDraft(
  draftKey: string,
  channelId: string | null,
  raw: string,
  text: string,
  mentionRefs: DraftMentionRef[],
) {
  const existing = loadDraftEntry(draftKey);
  const now = new Date().toISOString();
  const base = existing?.content ?? "";
  const content =
    replaceLastOccurrence(base, raw, text) ??
    (base.trim() ? `${base.replace(/\s+$/, "")} ${text}` : text);
  saveDraftEntry(draftKey, {
    channelId: existing?.channelId ?? channelId ?? draftKey,
    content,
    createdAt: existing?.createdAt ?? now,
    mentionRefs: [...(existing?.mentionRefs ?? []), ...mentionRefs],
    pendingImeta: existing?.pendingImeta ?? [],
    selectionStart: content.length,
    selectionEnd: content.length,
    spoileredAttachmentUrls: existing?.spoileredAttachmentUrls ?? [],
    status: "active",
    updatedAt: now,
  });
}

/**
 * Composer dictation controller: record → local transcript → raw words at the
 * caret (greyed) → cleanup → Buzz-validated mentions → swap (one undo step).
 * Every async step is fenced by a generation so a cancelled or superseded
 * dictation can never write into the composer.
 */
export function useComposerDictation(params: UseComposerDictationParams) {
  const [status, setStatusState] = React.useState<DictationStatus>("idle");
  const [startedAt, setStartedAt] = React.useState(0);
  const statusRef = React.useRef<DictationStatus>("idle");
  const paramsRef = React.useRef(params);
  paramsRef.current = params;
  const generationRef = React.useRef(0);
  const sessionRef = React.useRef<Session | null>(null);
  const pendingMarkRef = React.useRef<number | null>(null);
  const targetIdRef = React.useRef(0);
  const focusedAtRef = React.useRef(0);
  const mountedRef = React.useRef(true);
  const levelRef = React.useRef(0);
  const startedAtRef = React.useRef(0);
  const warnedRef = React.useRef(false);

  const setStatus = React.useCallback((next: DictationStatus) => {
    statusRef.current = next;
    if (mountedRef.current) setStatusState(next);
  }, []);

  const sameContext = React.useCallback((session: Session) => {
    const current = paramsRef.current;
    return (
      mountedRef.current &&
      current.draftKey === session.originDraftKey &&
      current.editTargetId === session.originEditTargetId &&
      current.editor !== null &&
      !current.editor.isDestroyed
    );
  }, []);

  const clearPendingMark = React.useCallback(() => {
    const editor = paramsRef.current.editor;
    if (editor && !editor.isDestroyed) {
      releaseDictationMark(editor.view, pendingMarkRef.current);
    }
    pendingMarkRef.current = null;
  }, []);

  const finish = React.useCallback(() => {
    sessionRef.current = null;
    clearPendingMark();
    setStatus("idle");
  }, [clearPendingMark, setStatus]);

  const cancel = React.useCallback(() => {
    const session = sessionRef.current;
    generationRef.current += 1;
    sessionRef.current = null;
    if (session) {
      session.capture?.cancel();
      if (session.backendId !== null)
        void cancelDictation(session.backendId).catch(() => {});
    }
    releaseRecorder(targetIdRef.current);
    clearPendingMark();
    setStatus("idle");
  }, [clearPendingMark, setStatus]);

  const cleanUp = React.useCallback(async (session: Session, raw: string) => {
    const formattedRaw = applySpokenFormatting(raw);
    const verbatim = () => resolveVerbatim(formattedRaw, session.roster);
    const settings = session.settings;
    if (
      !settings ||
      settings.cleanup.kind === "off" ||
      !settings.cloudCleanupEnabled ||
      session.backendId === null
    ) {
      if (session.backendId !== null)
        void cancelDictation(session.backendId).catch(() => {});
      return { segments: verbatim(), note: null as string | null };
    }
    try {
      const result = await cleanupDictation({
        sessionId: session.backendId,
        transcript: raw,
        roster: session.modelNames,
        vocabulary: [],
      });
      if (result.text) {
        return {
          segments: resolveModelOutput(
            applySpokenFormatting(result.text),
            session.roster,
          ),
          note: null,
        };
      }
      return {
        segments: verbatim(),
        note: result.failure ? describeCleanupFailure(result.failure) : null,
      };
    } catch {
      return {
        segments: verbatim(),
        note: "Cleanup failed, so here are your words as spoken.",
      };
    }
  }, []);

  const deliver = React.useCallback(
    (
      session: Session,
      raw: string,
      segments: DictationSegment[],
      note: string | null,
    ) => {
      if (sameContext(session)) {
        const editor = paramsRef.current.editor;
        if (!editor) return;
        const view = editor.view;
        const pendingId = pendingMarkRef.current;
        const mark = readDictationMark(view, pendingId);
        if (!mark) {
          if (pendingId !== null) {
            toast.info(
              "You edited the dictated words, so cleanup was not applied.",
            );
          }
          return;
        }
        const rendered = renderSegments(
          segments,
          paramsRef.current.registerMention,
        );
        const start = replaceRange(view, mark.from, mark.to, rendered.text);
        pendingMarkRef.current = null;
        if (start === null) return;
        for (const span of rendered.spans) {
          const id = trackDictationMark(view, {
            from: start + span.from,
            to: start + span.to,
            kind: span.kind === "mention" ? "flash" : "suggestion",
            entryKey: span.entryKey,
          });
          if (span.kind === "mention" && id !== null) {
            window.setTimeout(() => releaseDictationMark(view, id), FLASH_MS);
          }
        }
        if (note) toast.info(note);
        return;
      }
      if (session.originEditTargetId) {
        toast.info("Dictation was discarded because the message edit closed.");
        return;
      }
      if (!session.originDraftKey) return;
      const rendered = renderSegments(segments, (displayName) => displayName);
      deliverToDraft(
        session.originDraftKey,
        session.originChannelId,
        raw,
        rendered.text,
        rendered.mentionRefs,
      );
      toast.info("Your dictation was added to the draft where you started it.");
    },
    [sameContext],
  );

  const stop = React.useCallback(async () => {
    const session = sessionRef.current;
    if (!session) return;
    if (statusRef.current === "starting") {
      session.stopRequested = true;
      return;
    }
    if (statusRef.current !== "recording" || session.backendId === null) return;
    const generation = session.generation;
    setStatus("transcribing");
    let rawText: string;
    try {
      await session.capture?.stop();
      rawText = (await stopDictation(session.backendId)).text.trim();
    } catch (error) {
      releaseRecorder(targetIdRef.current);
      if (generation !== generationRef.current) return;
      void cancelDictation(session.backendId).catch(() => {});
      finish();
      showStartError(error);
      return;
    }
    releaseRecorder(targetIdRef.current);
    if (generation !== generationRef.current) return;
    if (!rawText) {
      void cancelDictation(session.backendId).catch(() => {});
      finish();
      toast.info(
        "Didn't catch anything. Try again a little closer to the mic.",
      );
      return;
    }
    if (sameContext(session)) {
      const editor = paramsRef.current.editor;
      const range = editor ? insertAtSelection(editor.view, rawText) : null;
      if (editor && range) {
        pendingMarkRef.current = trackDictationMark(editor.view, {
          ...range,
          kind: "pending",
        });
      }
    }
    setStatus("cleaning");
    const { segments, note } = await cleanUp(session, rawText);
    if (generation !== generationRef.current) return;
    deliver(session, rawText, segments, note);
    sessionRef.current = null;
    pendingMarkRef.current = null;
    setStatus("idle");
  }, [cleanUp, deliver, finish, sameContext, setStatus]);

  const start = React.useCallback(async () => {
    const current = paramsRef.current;
    if (!current.enabled || current.disabled || !current.editor) return;
    if (statusRef.current === "recording") {
      void stop();
      return;
    }
    if (statusRef.current !== "idle") return;
    if (current.huddleActive) {
      toast.info("Dictation is off during a huddle.");
      return;
    }
    const targetId = targetIdRef.current;
    if (!claimRecorder(targetId)) {
      toast.error("Dictation is already recording in another composer.");
      return;
    }
    generationRef.current += 1;
    const generation = generationRef.current;
    const session: Session = {
      generation,
      backendId: null,
      capture: null,
      roster: [],
      modelNames: [],
      settings: null,
      originDraftKey: current.draftKey,
      originChannelId: current.channelId,
      originEditTargetId: current.editTargetId,
      stopRequested: false,
    };
    sessionRef.current = session;
    setStatus("starting");
    try {
      const [info, settings] = await Promise.all([
        getDictationModelInfo(),
        getDictationSettings(),
      ]);
      if (info.status !== "ready") {
        throw new DictationSetupError(
          "Set up dictation first: download its speech model in Settings → Experiments → Dictation.",
        );
      }
      const candidates = current.getCandidates();
      session.roster = buildDictationRoster({
        ...candidates,
        recentPubkeys: current.recentMentionPubkeys,
      });
      session.modelNames = modelRosterNames(session.roster);
      session.settings = settings;
      const { sessionId } = await startDictation({
        roster: session.modelNames,
        vocabulary: [],
      });
      session.backendId = sessionId;
      if (generation !== generationRef.current) {
        void cancelDictation(sessionId).catch(() => {});
        return;
      }
      const capture = await startDictationCapture({
        deviceId: readStoredDevice(),
        onLevel: (level) => {
          levelRef.current = level;
        },
        onSilence: () =>
          toast.warning(
            "No audio from your microphone. Check your input device.",
          ),
      });
      if (generation !== generationRef.current) {
        capture.cancel();
        void cancelDictation(sessionId).catch(() => {});
        return;
      }
      session.capture = capture;
      startedAtRef.current = Date.now();
      warnedRef.current = false;
      if (mountedRef.current) setStartedAt(startedAtRef.current);
      setStatus("recording");
      if (session.stopRequested) void stop();
    } catch (error) {
      if (generation !== generationRef.current) return;
      if (session.backendId !== null)
        void cancelDictation(session.backendId).catch(() => {});
      releaseRecorder(targetId);
      sessionRef.current = null;
      setStatus("idle");
      showStartError(error);
    }
  }, [setStatus, stop]);

  const toggle = React.useCallback(() => {
    if (statusRef.current === "recording" || statusRef.current === "starting") {
      void stop();
    } else {
      void start();
    }
  }, [start, stop]);

  const isBusy = React.useCallback(() => statusRef.current !== "idle", []);
  const getLevel = React.useCallback(() => levelRef.current, []);
  const notifyBusy = React.useCallback(() => {
    toast.info("Finishing dictation…", { id: "dictation-busy" });
  }, []);

  // Register as a hotkey target for this composer's lifetime.
  React.useEffect(() => {
    const { id, unregister } = registerDictationTarget({
      isMain: paramsRef.current.isMain,
      isFocused: () => Boolean(paramsRef.current.editor?.isFocused),
      focusedAt: () => focusedAtRef.current,
      status: () => statusRef.current,
      start: () => void start(),
      stop: () => void stop(),
      cancel,
    });
    targetIdRef.current = id;
    return unregister;
  }, [cancel, start, stop]);

  // Track focus (hotkey routing) and suggestion clicks on this editor.
  const editor = params.editor;
  React.useEffect(() => {
    if (!editor || editor.isDestroyed) return;
    const onFocus = () => {
      focusedAtRef.current = Date.now();
    };
    const onSuggestion = (event: Event) => {
      const id = (event as CustomEvent<{ id: number }>).detail?.id;
      const mark = readDictationMark(editor.view, id ?? null);
      if (!mark) return;
      releaseDictationMark(editor.view, id ?? null);
      suggestionToPickerQuery(editor.view, mark.from, mark.to);
      editor.commands.focus();
    };
    editor.on("focus", onFocus);
    const dom = editor.view.dom;
    dom.addEventListener(DICTATION_SUGGESTION_EVENT, onSuggestion);
    return () => {
      editor.off("focus", onFocus);
      dom.removeEventListener(DICTATION_SUGGESTION_EVENT, onSuggestion);
    };
  }, [editor]);

  // Leaving the composer's context mid-recording stops and delivers to the
  // draft where dictation started.
  // biome-ignore lint/correctness/useExhaustiveDependencies: identity fields are the triggers
  React.useEffect(() => {
    if (statusRef.current === "recording" || statusRef.current === "starting") {
      void stop();
    }
  }, [params.draftKey, params.editTargetId]);

  // A huddle takes the mic: dictation yields immediately.
  React.useEffect(() => {
    if (
      params.huddleActive &&
      (statusRef.current === "recording" || statusRef.current === "starting")
    ) {
      cancel();
      toast.info("Dictation stopped because a huddle started.");
    }
  }, [cancel, params.huddleActive]);

  // The 5-minute cap and its warning. Waveform and clock live in the button
  // (via getLevel/startedAt) so the composer does not re-render per tick.
  React.useEffect(() => {
    if (status !== "recording") return;
    const timer = window.setInterval(() => {
      const seconds = Math.floor((Date.now() - startedAtRef.current) / 1000);
      if (seconds >= DICTATION_WARN_SECONDS && !warnedRef.current) {
        warnedRef.current = true;
        toast.info("30 seconds of dictation left.");
      }
      if (seconds >= DICTATION_MAX_SECONDS) void stop();
    }, 500);
    return () => window.clearInterval(timer);
  }, [status, stop]);

  // Unmounting mid-recording discards; mid-cleanup still delivers to the draft.
  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (
        statusRef.current === "recording" ||
        statusRef.current === "starting"
      ) {
        cancel();
      }
    };
  }, [cancel]);

  return {
    status,
    startedAt,
    getLevel,
    notifyBusy,
    start,
    stop,
    toggle,
    cancel,
    isBusy,
  };
}

export type ComposerDictation = ReturnType<typeof useComposerDictation>;
