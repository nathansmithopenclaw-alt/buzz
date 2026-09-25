import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { getSchema } from "@tiptap/core";
import { EditorState, TextSelection } from "@tiptap/pm/state";
import StarterKit from "@tiptap/starter-kit";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
  pretendToBeVisual: true,
});

// ── Backend (Tauri IPC) stub ─────────────────────────────────────────────────

const invoked = [];
let handlers = {};
function deferred() {
  let resolve;
  const promise = new Promise((r) => {
    resolve = r;
  });
  return { promise, resolve };
}
const tauriInternals = {
  invoke: (command, args) => {
    invoked.push(command);
    const handler = handlers[command];
    if (!handler) return Promise.reject(new Error(`unmocked: ${command}`));
    return Promise.resolve().then(() => handler(args));
  },
  transformCallback: () => 1,
};

// ── Microphone / Web Audio stub ──────────────────────────────────────────────

class FakeAudioContext {
  state = "running";
  sampleRate = 48_000;
  audioWorklet = { addModule: async () => {} };
  createMediaStreamSource() {
    return { connect() {}, disconnect() {} };
  }
  close() {
    return Promise.resolve();
  }
}
class FakeAudioWorkletNode {
  port = { onmessage: null };
  disconnect() {}
}

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
    localStorage: dom.window.localStorage,
    __TAURI_INTERNALS__: tauriInternals,
    AudioContext: FakeAudioContext,
    AudioWorkletNode: FakeAudioWorkletNode,
  });
  dom.window.__TAURI_INTERNALS__ = tauriInternals;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      platform: "MacIntel",
      userAgent: "node",
      mediaDevices: {
        getUserMedia: async () => ({ getTracks: () => [{ stop() {} }] }),
      },
    },
  });
});

after(() => dom.window.close());

// ── A composer editor over a real EditorState with the real marks plugin ─────

let lib;
async function loadLib() {
  if (lib) return lib;
  const { DictationMarksExtension } = await import("./lib/dictationMarks.ts");
  const schema = getSchema([
    StarterKit.configure({ heading: false, trailingNode: false, link: false }),
  ]);
  const plugins = DictationMarksExtension.config.addProseMirrorPlugins.call({});
  lib = { schema, plugins };
  return lib;
}

async function makeEditor(text = "") {
  const { schema, plugins } = await loadLib();
  const doc = schema.nodes.doc.create(null, [
    schema.nodes.paragraph.create(null, text ? [schema.text(text)] : []),
  ]);
  let state = EditorState.create({ doc, plugins });
  // The caret sits at the end of the existing text, as when typing.
  state = state.apply(state.tr.setSelection(TextSelection.atEnd(state.doc)));
  const view = {
    state,
    dom: dom.window.document.createElement("div"),
    isDestroyed: false,
    dispatch(tr) {
      view.state = view.state.apply(tr);
    },
  };
  return {
    view,
    isDestroyed: false,
    isFocused: false,
    on() {},
    off() {},
    commands: { focus() {} },
    text: () =>
      view.state.doc.textBetween(0, view.state.doc.content.size, "\n", "\n"),
  };
}

function backend({ cleanup, raw = "hello there" } = {}) {
  handlers = {
    dictation_model_status: () => ({
      status: "ready",
      requiredBytes: 1,
      freeBytes: 2,
      enoughSpace: true,
    }),
    get_dictation_settings: () => ({
      version: 1,
      cleanup: { kind: "connection", connectionId: "c1", model: "m" },
      timeoutMs: 4000,
      hotkey: "AltRight",
      language: "en",
      cloudCleanupEnabled: true,
    }),
    dictation_start: () => ({ sessionId: 7 }),
    dictation_push_audio: () => null,
    dictation_stop: () => ({
      text: raw,
      audioMs: 1000,
      decodeMs: 10,
      tailDecodeMs: 5,
      chunks: 1,
    }),
    dictation_cancel: () => null,
    dictation_cleanup: () =>
      cleanup ?? { text: "Hello there.", failure: null, elapsedMs: 5 },
  };
}

const emptyCandidates = () => ({
  candidates: [],
  currentPubkey: null,
  memberPubkeys: new Set(),
});

async function mountController(editor, overrides = {}) {
  const { act, renderHook } = await import("@testing-library/react");
  const { useComposerDictation } = await import("./useComposerDictation.ts");
  const base = {
    enabled: true,
    editor,
    draftKey: "draft-a",
    channelId: "channel-a",
    editTargetId: null,
    isMain: true,
    disabled: false,
    huddleActive: false,
    voiceNoteActive: false,
    getCandidates: emptyCandidates,
    registerMention: (name) => name,
    ...overrides,
  };
  const hook = renderHook((props) => useComposerDictation(props), {
    initialProps: base,
  });
  const set = async (patch) => {
    await act(async () => {
      hook.rerender({ ...base, ...patch });
      await flush();
    });
    Object.assign(base, patch);
  };
  return { hook, act, set };
}

async function flush(times = 8) {
  for (let i = 0; i < times; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

async function recordAndHold(hook, act, cleanupGate) {
  await act(async () => {
    void hook.result.current.start();
    await flush();
  });
  assert.equal(hook.result.current.status, "recording");
  if (cleanupGate) {
    handlers.dictation_cleanup = () => cleanupGate.promise;
  }
}

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  invoked.length = 0;
  const { resetDictationState } = await import("./lib/dictationSession.ts");
  resetDictationState();
  await flush();
});

// ── Tests ────────────────────────────────────────────────────────────────────

test("cleaned text replaces the raw words in place when the composer stays put", async () => {
  backend();
  const editor = await makeEditor("Hi");
  const { hook, act } = await mountController(editor);
  await recordAndHold(hook, act);
  await act(async () => {
    await hook.result.current.stop();
    await flush();
  });
  assert.equal(editor.text(), "Hi Hello there.");
  assert.equal(hook.result.current.status, "idle");
});

test("leaving and returning to the draft during cleanup still delivers the dictation", async () => {
  backend();
  const gate = deferred();
  const editor = await makeEditor("");
  const { hook, act, set } = await mountController(editor);
  await recordAndHold(hook, act, gate);
  // A → B mid-recording: stops, transcribes in B (raw words not inserted).
  await set({ draftKey: "draft-b", channelId: "channel-b" });
  assert.equal(hook.result.current.status, "cleaning");
  assert.equal(editor.text(), "", "raw words are not dropped into draft B");
  // B → A before cleanup lands.
  await set({ draftKey: "draft-a", channelId: "channel-a" });
  await act(async () => {
    gate.resolve({ text: "Hello there.", failure: null, elapsedMs: 5 });
    await flush();
  });
  assert.equal(editor.text(), "Hello there.");
  assert.equal(hook.result.current.status, "idle");
});

test("a result waits while an edit is open over its draft, then lands after the edit closes", async () => {
  backend();
  const gate = deferred();
  const editor = await makeEditor("");
  const { hook, act, set } = await mountController(editor);
  await recordAndHold(hook, act, gate);
  const { initDraftStore, loadDraftEntry } = await import(
    "@/features/messages/lib/useDrafts"
  );
  initDraftStore("pk-test");
  // An edit opens over draft A mid-recording.
  await set({ editTargetId: "msg-1" });
  await act(async () => {
    gate.resolve({ text: "Hello there.", failure: null, elapsedMs: 5 });
    await flush();
  });
  assert.equal(editor.text(), "", "nothing lands inside the edit");
  assert.equal(
    loadDraftEntry("draft-a"),
    undefined,
    "nothing is written under the composer's in-memory draft",
  );
  assert.equal(hook.result.current.status, "idle", "sends are not held");
  // The edit closes; the composer restores draft A, then the result lands.
  await set({ editTargetId: null });
  await act(async () => {
    await new Promise((resolve) => dom.window.requestAnimationFrame(resolve));
    await flush();
  });
  assert.equal(editor.text(), "Hello there.");
});

test("a composer that moved to another draft appends to the saved origin draft, never rewriting the user's own words", async () => {
  backend({ raw: "hello there" });
  const gate = deferred();
  const editor = await makeEditor("");
  const { hook, act, set } = await mountController(editor);
  const { initDraftStore, loadDraftEntry, saveDraftEntry } = await import(
    "@/features/messages/lib/useDrafts"
  );
  initDraftStore("pk-test-2");
  await recordAndHold(hook, act, gate);
  await set({ draftKey: "draft-b", channelId: "channel-b" });
  // What the composer persisted for draft A when it left.
  saveDraftEntry("draft-a", {
    channelId: "channel-a",
    // The user typed these words themselves; the dictation never reached
    // this draft's editor, so they must not be mistaken for it.
    content: "I said hello there",
    createdAt: "2026-09-24T00:00:00.000Z",
    mentionRefs: [],
    pendingImeta: [],
    selectionStart: 18,
    selectionEnd: 18,
    spoileredAttachmentUrls: [],
    status: "active",
    updatedAt: "2026-09-24T00:00:00.000Z",
  });
  await act(async () => {
    gate.resolve({ text: "Hello there.", failure: null, elapsedMs: 5 });
    await flush();
  });
  assert.equal(
    loadDraftEntry("draft-a")?.content,
    "I said hello there Hello there.",
  );
  assert.equal(editor.text(), "", "draft B's editor is untouched");
});

test("sends are held only for the draft the dictation belongs to", async () => {
  backend();
  const gate = deferred();
  const editor = await makeEditor("");
  const { hook, act, set } = await mountController(editor);
  await recordAndHold(hook, act, gate);
  assert.equal(hook.result.current.isBusy(), true);
  await set({ draftKey: "draft-b", channelId: "channel-b" });
  assert.equal(hook.result.current.status, "cleaning");
  assert.equal(hook.result.current.isBusy(), false);
  assert.equal(hook.result.current.blocksSend, false);
  await act(async () => {
    gate.resolve({ text: "x", failure: null, elapsedMs: 1 });
    await flush();
  });
});

test("a start cancelled while loading settings never opens a backend session", async () => {
  backend();
  const gate = deferred();
  handlers.dictation_model_status = () => gate.promise;
  const editor = await makeEditor("");
  const { hook, act } = await mountController(editor);
  await act(async () => {
    void hook.result.current.start();
    await flush();
  });
  await act(async () => {
    hook.result.current.cancel();
    gate.resolve({
      status: "ready",
      requiredBytes: 1,
      freeBytes: 2,
      enoughSpace: true,
    });
    await flush();
  });
  assert.equal(invoked.includes("dictation_start"), false);
  assert.equal(hook.result.current.status, "idle");
});

test("a voice note owns the mic: dictation refuses to start", async () => {
  backend();
  const editor = await makeEditor("");
  const { hook, act } = await mountController(editor, {
    voiceNoteActive: true,
  });
  await act(async () => {
    await hook.result.current.start();
    await flush();
  });
  assert.equal(hook.result.current.status, "idle");
  assert.equal(invoked.length, 0);
});

test("cloud cleanup off still asks the backend, which can clean up locally", async () => {
  backend();
  handlers.get_dictation_settings = () => ({
    version: 1,
    cleanup: { kind: "connection", connectionId: "c1", model: "m" },
    timeoutMs: 4000,
    hotkey: "AltRight",
    language: "en",
    cloudCleanupEnabled: false,
  });
  const editor = await makeEditor("");
  const { hook, act } = await mountController(editor);
  await recordAndHold(hook, act);
  await act(async () => {
    await hook.result.current.stop();
    await flush();
  });
  assert.equal(invoked.includes("dictation_cleanup"), true);
  assert.equal(editor.text(), "Hello there.");
});

test("a not_configured cleanup result stays silent: words as spoken, no note", async () => {
  backend({
    cleanup: { text: null, failure: { kind: "not_configured" }, elapsedMs: 1 },
  });
  handlers.get_dictation_settings = () => ({
    version: 1,
    cleanup: { kind: "agent_open_router", model: "m" },
    timeoutMs: 4000,
    hotkey: "AltRight",
    language: "en",
    cloudCleanupEnabled: false,
  });
  const editor = await makeEditor("");
  const { hook, act } = await mountController(editor);
  const { toast } = await import("sonner");
  const notes = [];
  const originalInfo = toast.info;
  toast.info = (message) => notes.push(message);
  try {
    await recordAndHold(hook, act);
    await act(async () => {
      await hook.result.current.stop();
      await flush();
    });
  } finally {
    toast.info = originalInfo;
  }
  assert.equal(invoked.includes("dictation_cleanup"), true);
  assert.equal(editor.text(), "hello there");
  assert.deepEqual(notes, [], "not_configured must not surface a note");
});

test("a failure while delivering never strands the composer in a busy state", async () => {
  backend();
  const editor = await makeEditor("");
  const { hook, act } = await mountController(editor);
  await recordAndHold(hook, act);
  // Raw words insert fine; the swap then throws.
  const originalDispatch = editor.view.dispatch;
  let dispatches = 0;
  editor.view.dispatch = (tr) => {
    dispatches += 1;
    if (dispatches === 3) throw new Error("editor exploded");
    originalDispatch(tr);
  };
  await act(async () => {
    await hook.result.current.stop();
    await flush();
  });
  assert.equal(hook.result.current.status, "idle");
  assert.equal(hook.result.current.isBusy(), false);
  assert.match(editor.text(), /hello there/, "the raw words are kept");
});
