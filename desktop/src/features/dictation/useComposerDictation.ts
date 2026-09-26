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
  type DictationSettings,
  getDictationModelInfo,
  getDictationSettings,
  noteDictationCapture,
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
  findTextRanges,
  insertAtSelection,
  replaceRange,
  suggestionToPickerQuery,
} from "./lib/editorOps";
import {
  type DictationSegment,
  resolveVerbatim,
} from "./lib/mentionResolution";
import { acceptPolish, toPolishRequest } from "./lib/polish";
import { type RegisterMention, renderSegments } from "./lib/renderSegments";
import {
  applySpokenCorrections,
  applySpokenFormatting,
} from "./lib/spokenFormatting";

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
  /** Local capture of this dictation, when captures are on. */
  captureId: string | null;
};

/** A finished dictation waiting for (or being handed to) its destination. */
type Outcome = {
  session: Session;
  segments: DictationSegment[];
  /** Where it landed in the live editor, if it did. */
  inserted: { from: number; to: number; text: string } | null;
};

/** Extra time past the polish timeout before the result is ignored. */
const POLISH_BACKSTOP_GRACE_MS = 5000;
const DEFAULT_POLISH_TIMEOUT_MS = 4000;

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

/** Most captures remembered per draft until it is sent. */
const MAX_DRAFT_CAPTURES = 20;

/** Records what happened to a dictation in its local capture, if any. */
function noteCapture(captureId: string | null, note: Record<string, unknown>) {
  if (captureId) void noteDictationCapture(captureId, note).catch(() => {});
}

/**
 * Adds dictated text to a saved draft whose composer is no longer showing
 * it. A mention whose label another identity already holds in this draft is
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
  const content = base.trim()
    ? `${base.replace(/\s+$/, "")} ${rendered.text}`
    : rendered.text;
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
 * Composer dictation controller: record → local transcript (roster names
 * already spelled right) → local corrections and mention resolution → words
 * and mentions at the caret, final and sendable at once. An optional polish
 * pass may then tidy the words in place, but only while the user has not
 * touched them, and it can never add or remove a mention.
 *
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
  /** Captures inserted into each draft since its last send. */
  const draftCapturesRef = React.useRef(new Map<string, string[]>());
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

  const cancel = React.useCallback(() => {
    const session = sessionRef.current;
    generationRef.current += 1;
    sessionRef.current = null;
    if (session) {
      session.capture?.cancel();
      releaseBackend(session.backendId);
    }
    releaseRecorder(targetIdRef.current);
    setStatus("idle");
  }, [setStatus]);

  /**
   * Tidies an inserted dictation with the configured source. The swap happens
   * only while its words are exactly as inserted; otherwise, or when the
   * reply does anything but polish, the dictation stays as it is.
   */
  const polishInPlace = React.useCallback(async (outcome: Outcome) => {
    const { session, inserted } = outcome;
    const settings = session.settings;
    const editor = paramsRef.current.editor;
    if (
      !inserted ||
      !editor ||
      !settings ||
      settings.cleanup.kind === "off" ||
      session.backendId === null
    ) {
      releaseBackend(session.backendId);
      return;
    }
    const view = editor.view;
    const markId = trackDictationMark(view, {
      from: inserted.from,
      to: inserted.to,
      kind: "polish",
    });
    const request = toPolishRequest(outcome.segments);
    let backstop: number | undefined;
    try {
      const deadline = new Promise<"backstop">((resolve) => {
        backstop = window.setTimeout(
          () => resolve("backstop"),
          (settings.timeoutMs || DEFAULT_POLISH_TIMEOUT_MS) +
            POLISH_BACKSTOP_GRACE_MS,
        );
      });
      const result = await Promise.race([
        cleanupDictation({
          sessionId: session.backendId,
          transcript: request.text,
          vocabulary: [],
        }),
        deadline,
      ]);
      if (result === "backstop") {
        releaseBackend(session.backendId);
        noteCapture(session.captureId, { kind: "polish", outcome: "timeout" });
        return;
      }
      if (!result.text) {
        noteCapture(session.captureId, {
          kind: "polish",
          outcome: "failed",
          failure: result.failure,
          elapsedMs: result.elapsedMs,
        });
        return;
      }
      const verdict = acceptPolish(request, result.text);
      // The mark follows the words through edits around them. If the
      // composer rewrote the document without changing them, the exact
      // words, found exactly once, still identify the dictation.
      const marked = readDictationMark(view, markId);
      const markIntact =
        marked !== null &&
        view.state.doc.textBetween(marked.from, marked.to, "\n", "\n") ===
          inserted.text;
      const found = markIntact
        ? []
        : findTextRanges(view.state.doc, inserted.text);
      const range = markIntact ? marked : found.length === 1 ? found[0] : null;
      noteCapture(session.captureId, {
        kind: "polish",
        input: request.text,
        reply: result.text,
        elapsedMs: result.elapsedMs,
        markAlive: marked !== null,
        outcome: !verdict.accepted
          ? `rejected: ${verdict.reason}`
          : range === null
            ? "skipped: edited or sent"
            : markIntact
              ? "applied"
              : "applied: found by text",
      });
      if (!verdict.accepted || !range || editor.isDestroyed) return;
      const rendered = renderSegments(
        verdict.segments,
        paramsRef.current.registerMention,
      );
      if (rendered.text === inserted.text) return;
      const start = replaceRange(view, range.from, range.to, rendered.text);
      if (start === null) return;
      for (const span of rendered.spans) {
        if (span.kind !== "suggestion") continue;
        trackDictationMark(view, {
          from: start + span.from,
          to: start + span.to,
          kind: "suggestion",
          entryKey: span.entryKey,
        });
      }
    } catch {
      noteCapture(session.captureId, { kind: "polish", outcome: "error" });
    } finally {
      window.clearTimeout(backstop);
      if (!editor.isDestroyed) releaseDictationMark(view, markId);
    }
  }, []);

  /** Puts a result into the live editor at the caret. */
  const applyInEditor = React.useCallback((outcome: Outcome) => {
    const editor = paramsRef.current.editor;
    if (!editor || editor.isDestroyed) return;
    const view = editor.view;
    const rendered = renderSegments(
      outcome.segments,
      paramsRef.current.registerMention,
    );
    const range = insertAtSelection(view, rendered.text);
    if (!range) return;
    const start = range.from;
    outcome.inserted = { ...range, text: rendered.text };
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
      const transcript = await stopDictation(backendId);
      rawText = transcript.text.trim();
      session.captureId = transcript.captureId ?? null;
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
    const text = applySpokenFormatting(applySpokenCorrections(rawText));
    const outcome: Outcome = {
      session,
      segments: resolveVerbatim(text, session.roster),
      inserted: null,
    };
    sessionRef.current = null;
    setStatus("idle");
    try {
      if (deliver(outcome) === "parked") parkedRef.current = outcome;
    } catch {
      toast.error("Dictation could not be inserted.");
      releaseBackend(backendId);
      return;
    }
    if (session.captureId) {
      noteCapture(session.captureId, {
        kind: "inserted",
        text: outcome.inserted?.text ?? null,
        mentions: outcome.segments.flatMap((s) =>
          s.type === "mention" ? [s.entry.displayName] : [],
        ),
        suggestions: outcome.segments.flatMap((s) =>
          s.type === "suggestion" ? [s.entry.displayName] : [],
        ),
      });
      if (session.originDraftKey) {
        const ids = draftCapturesRef.current.get(session.originDraftKey) ?? [];
        draftCapturesRef.current.set(
          session.originDraftKey,
          [...ids, session.captureId].slice(-MAX_DRAFT_CAPTURES),
        );
      }
    }
    void polishInPlace(outcome);
  }, [deliver, polishInPlace, setStatus]);

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
      captureId: null,
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
          "Set up dictation first: download its speech model in Settings → Voice.",
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
  /** Records the sent message in the captures dictated into this draft. */
  const noteSent = React.useCallback((content: string) => {
    const draftKey = paramsRef.current.draftKey;
    if (!draftKey) return;
    const ids = draftCapturesRef.current.get(draftKey);
    if (!ids) return;
    draftCapturesRef.current.delete(draftKey);
    for (const id of ids) noteCapture(id, { kind: "sent", content });
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
    noteSent,
    start,
    stop,
    toggle,
    cancel,
    isBusy,
  };
}

export type ComposerDictation = ReturnType<typeof useComposerDictation>;
