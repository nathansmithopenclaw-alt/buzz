import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Editor } from "@tiptap/core";
import { toast } from "sonner";

import type { MentionCandidate } from "@/features/messages/lib/mentionCandidates";
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
  DICTATION_DEVICE_STORAGE_KEY,
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
  suggestionKeyLabel,
  trackDictationMark,
} from "./lib/dictationMarks";
import {
  backendIdle,
  claimRecorder,
  type DictationStatus,
  registerDictationTarget,
  releaseRecorder,
  trackBackendRelease,
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
  /** A voice note is recording or processing (it owns the mic). */
  voiceNoteActive: boolean;
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
  /** The raw words were put in the editor (under `rawMarkId`). */
  rawInserted: boolean;
  rawMarkId: number | null;
};

/** A finished dictation waiting for (or being handed to) its destination. */
type Outcome = {
  session: Session;
  raw: string;
  segments: DictationSegment[];
  note: string | null;
};

/** Extra time past the cleanup timeout before the UI stops waiting. */
const CLEANUP_BACKSTOP_GRACE_MS = 5000;
const DEFAULT_CLEANUP_TIMEOUT_MS = 4000;

function readStoredDevice(): string | null {
  try {
    return window.localStorage.getItem(DICTATION_DEVICE_STORAGE_KEY);
  } catch {
    return null;
  }
}

/** Ends a backend session; a later start waits until this has landed. */
function releaseBackend(sessionId: number | null) {
  if (sessionId === null) return;
  trackBackendRelease(cancelDictation(sessionId));
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

/**
 * Adds dictated text to a saved draft whose composer is no longer showing
 * it. Words that were already inserted before the composer moved away are
 * swapped in place (whole-word match); otherwise the text is appended. A
 * mention whose label another identity already holds in this draft is
 * written as a plain name, so restoring the draft never rebinds it.
 */
function deliverToDraft(
  draftKey: string,
  channelId: string | null,
  outcome: Outcome,
) {
  const existing = loadDraftEntry(draftKey);
  const boundLabels = new Map<string, string>();
  for (const ref of existing?.mentionRefs ?? [])
    boundLabels.set(ref.displayName, ref.pubkey);
  const rendered = renderSegments(outcome.segments, (displayName, pubkey) => {
    const holder = boundLabels.get(displayName);
    if (holder && holder !== pubkey) return null;
    boundLabels.set(displayName, pubkey);
    return displayName;
  });
  const now = new Date().toISOString();
  const base = existing?.content ?? "";
  const swapped = outcome.session.rawInserted
    ? replaceLastOccurrence(base, outcome.raw, rendered.text)
    : null;
  const content =
    swapped ??
    (base.trim()
      ? `${base.replace(/\s+$/, "")} ${rendered.text}`
      : rendered.text);
  const refs = [...(existing?.mentionRefs ?? [])];
  for (const ref of rendered.mentionRefs) {
    if (!refs.some((r) => r.displayName === ref.displayName)) refs.push(ref);
  }
  saveDraftEntry(draftKey, {
    channelId: existing?.channelId ?? channelId ?? draftKey,
    content,
    createdAt: existing?.createdAt ?? now,
    mentionRefs: refs,
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
 * dictation can never write into the composer. A result always reaches the
 * draft it was spoken into: the live editor, the saved draft once the
 * composer has moved on, or — while an edit is open over that same draft —
 * held until the composer returns to it.
 */
export function useComposerDictation(params: UseComposerDictationParams) {
  const [status, setStatusState] = React.useState<DictationStatus>("idle");
  const [startedAt, setStartedAt] = React.useState(0);
  const statusRef = React.useRef<DictationStatus>("idle");
  const paramsRef = React.useRef(params);
  paramsRef.current = params;
  const generationRef = React.useRef(0);
  const sessionRef = React.useRef<Session | null>(null);
  const parkedRef = React.useRef<Outcome | null>(null);
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

  const releaseMark = React.useCallback((id: number | null) => {
    const editor = paramsRef.current.editor;
    if (editor && !editor.isDestroyed) releaseDictationMark(editor.view, id);
  }, []);

  const cancel = React.useCallback(() => {
    const session = sessionRef.current;
    generationRef.current += 1;
    sessionRef.current = null;
    if (session) {
      session.capture?.cancel();
      releaseBackend(session.backendId);
      releaseMark(session.rawMarkId);
    }
    releaseRecorder(targetIdRef.current);
    setStatus("idle");
  }, [releaseMark, setStatus]);

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
      releaseBackend(session.backendId);
      return { segments: verbatim(), note: null as string | null };
    }
    let backstop: number | undefined;
    try {
      const deadline = new Promise<"backstop">((resolve) => {
        backstop = window.setTimeout(
          () => resolve("backstop"),
          (settings.timeoutMs || DEFAULT_CLEANUP_TIMEOUT_MS) +
            CLEANUP_BACKSTOP_GRACE_MS,
        );
      });
      const result = await Promise.race([
        cleanupDictation({
          sessionId: session.backendId,
          transcript: raw,
          roster: session.modelNames,
          vocabulary: [],
        }),
        deadline,
      ]);
      if (result === "backstop") {
        releaseBackend(session.backendId);
        return {
          segments: verbatim(),
          note: "Cleanup took too long, so here are your words as spoken.",
        };
      }
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
    } finally {
      window.clearTimeout(backstop);
    }
  }, []);

  /** Puts a result into the live editor (swap the raw words, or insert). */
  const applyInEditor = React.useCallback((outcome: Outcome) => {
    const editor = paramsRef.current.editor;
    if (!editor || editor.isDestroyed) return;
    const view = editor.view;
    const { session } = outcome;
    let range: { from: number; to: number } | null = null;
    if (session.rawInserted) {
      const mark = readDictationMark(view, session.rawMarkId);
      session.rawMarkId = null;
      if (!mark) {
        toast.info(
          "The dictated words changed before cleanup finished, so they were kept as spoken.",
        );
        return;
      }
      range = mark;
    }
    const rendered = renderSegments(
      outcome.segments,
      paramsRef.current.registerMention,
    );
    const start = range
      ? replaceRange(view, range.from, range.to, rendered.text)
      : (insertAtSelection(view, rendered.text)?.from ?? null);
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
    const suggestions = rendered.spans.filter(
      (span) => span.kind === "suggestion",
    ).length;
    if (suggestions > 0) {
      toast.info(
        `${suggestions === 1 ? "A name" : `${suggestions} names`} may be meant as a mention. Click the underlined ${suggestions === 1 ? "name" : "names"}, or put the caret in one and press ${suggestionKeyLabel()}.`,
      );
    }
    if (outcome.note) toast.info(outcome.note);
  }, []);

  /**
   * Hands a result to where it was spoken. `parked` means the composer still
   * shows the origin draft but an edit is open over it: the composer holds
   * that draft in memory and will restore it, so the result waits.
   */
  const deliver = React.useCallback(
    (outcome: Outcome): "done" | "parked" => {
      const { session } = outcome;
      if (sameContext(session)) {
        applyInEditor(outcome);
        return "done";
      }
      if (session.originEditTargetId) {
        toast.info("Dictation was discarded because the message edit closed.");
        return "done";
      }
      if (!session.originDraftKey) return "done";
      if (
        mountedRef.current &&
        paramsRef.current.draftKey === session.originDraftKey
      ) {
        return "parked";
      }
      deliverToDraft(session.originDraftKey, session.originChannelId, outcome);
      toast.info("Your dictation was added to the draft where you started it.");
      return "done";
    },
    [applyInEditor, sameContext],
  );

  /** Retries a held result after the composer's context changed. */
  const flushParked = React.useCallback(() => {
    const outcome = parkedRef.current;
    if (!outcome) return;
    const attempt = () => {
      if (parkedRef.current !== outcome) return;
      if (deliver(outcome) === "done") parkedRef.current = null;
    };
    // Back on the origin draft: let the composer restore its own content
    // for this context first (its effects run after ours in this commit).
    if (sameContext(outcome.session)) window.requestAnimationFrame(attempt);
    else attempt();
  }, [deliver, sameContext]);

  const stop = React.useCallback(async () => {
    const session = sessionRef.current;
    if (!session) return;
    if (statusRef.current === "starting") {
      session.stopRequested = true;
      return;
    }
    if (statusRef.current !== "recording" || session.backendId === null) return;
    const generation = session.generation;
    const backendId = session.backendId;
    setStatus("transcribing");
    let rawText: string;
    let droppedBatches = 0;
    try {
      droppedBatches = (await session.capture?.stop())?.droppedBatches ?? 0;
      rawText = (await stopDictation(backendId)).text.trim();
    } catch (error) {
      // A cancel in the meantime already released everything.
      if (generation !== generationRef.current) return;
      releaseRecorder(targetIdRef.current);
      releaseBackend(backendId);
      sessionRef.current = null;
      setStatus("idle");
      showStartError(error);
      return;
    }
    if (generation !== generationRef.current) return;
    releaseRecorder(targetIdRef.current);
    if (!rawText) {
      releaseBackend(backendId);
      sessionRef.current = null;
      setStatus("idle");
      toast.info(
        "Didn't catch anything. Try again a little closer to the mic.",
      );
      return;
    }
    if (droppedBatches > 0) {
      toast.warning(
        "Some audio didn't reach dictation, so part of what you said may be missing.",
      );
    }
    if (sameContext(session)) {
      const editor = paramsRef.current.editor;
      const range = editor ? insertAtSelection(editor.view, rawText) : null;
      if (editor && range) {
        session.rawInserted = true;
        session.rawMarkId = trackDictationMark(editor.view, {
          ...range,
          kind: "pending",
        });
      }
    }
    setStatus("cleaning");
    try {
      const { segments, note } = await cleanUp(session, rawText);
      if (generation !== generationRef.current) return;
      const outcome: Outcome = { session, raw: rawText, segments, note };
      if (deliver(outcome) === "parked") parkedRef.current = outcome;
    } catch {
      if (generation === generationRef.current) {
        toast.error(
          "Dictation could not finish, so your words were kept as spoken.",
        );
      }
    } finally {
      // Never strand the composer (sends are held while not idle).
      if (generation === generationRef.current) {
        releaseMark(session.rawMarkId);
        sessionRef.current = null;
        setStatus("idle");
      }
    }
  }, [cleanUp, deliver, releaseMark, sameContext, setStatus]);

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
    if (current.voiceNoteActive) {
      toast.info("Finish or discard the voice note before dictating.");
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
      rawInserted: false,
      rawMarkId: null,
    };
    sessionRef.current = session;
    setStatus("starting");
    // After each await: a cancel in the meantime (Esc, a typing chord, a
    // huddle) bumped the generation and already released what it could see.
    try {
      const [info, settings] = await Promise.all([
        getDictationModelInfo(),
        getDictationSettings(),
      ]);
      if (generation !== generationRef.current) return;
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
      // The backend has one session slot: wait out any stale session's
      // release instead of racing it.
      await backendIdle();
      if (generation !== generationRef.current) return;
      const started = startDictation({
        roster: session.modelNames,
        vocabulary: [],
      });
      // If this start lands after a cancel, it frees its own slot.
      trackBackendRelease(
        started.then(({ sessionId }) =>
          generation !== generationRef.current
            ? cancelDictation(sessionId)
            : undefined,
        ),
      );
      const { sessionId } = await started;
      if (generation !== generationRef.current) return;
      session.backendId = sessionId;
      const capture = await startDictationCapture({
        deviceId: readStoredDevice(),
        onLevel: (level) => {
          levelRef.current = level;
        },
        onSilence: () =>
          toast.warning(
            "No audio from your microphone. Check your input device.",
          ),
        onDeviceFallback: () =>
          toast.info(
            "Your chosen microphone isn't connected, so dictation is using the system default.",
          ),
      });
      if (generation !== generationRef.current) {
        capture.cancel();
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
      releaseBackend(session.backendId);
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

  /** Dictation in progress for the draft this composer is showing now. */
  const originMatches = React.useCallback(() => {
    const session = sessionRef.current;
    const current = paramsRef.current;
    return (
      session !== null &&
      session.originDraftKey === current.draftKey &&
      session.originEditTargetId === current.editTargetId
    );
  }, []);
  const isBusy = React.useCallback(
    () => statusRef.current !== "idle" && originMatches(),
    [originMatches],
  );
  const discard = React.useCallback(() => {
    parkedRef.current = null;
    cancel();
  }, [cancel]);
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
      discard,
    });
    targetIdRef.current = id;
    return unregister;
  }, [cancel, discard, start, stop]);

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
  // draft where dictation started; a held result gets another chance.
  // biome-ignore lint/correctness/useExhaustiveDependencies: identity fields are the triggers
  React.useEffect(() => {
    if (statusRef.current === "recording" || statusRef.current === "starting") {
      void stop();
    }
    flushParked();
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

  // Unmounting mid-recording discards; mid-cleanup still delivers to the
  // draft, and so does a held result (the composer has persisted it by now).
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
      flushParked();
    };
  }, [cancel, flushParked]);

  return {
    status,
    /** Render-time twin of `isBusy`: hold sends for this draft only. */
    blocksSend: status !== "idle" && originMatches(),
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
