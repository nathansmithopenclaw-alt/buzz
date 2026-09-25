import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

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

function installMedia(getUserMedia, devices = []) {
  Object.assign(globalThis, {
    AudioContext: FakeAudioContext,
    AudioWorkletNode: FakeAudioWorkletNode,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      mediaDevices: { getUserMedia, enumerateDevices: async () => devices },
    },
  });
}

const stream = () => ({ getTracks: () => [{ stop() {} }] });
const domError = (name) => Object.assign(new Error(name), { name });

afterEach(() => {
  delete globalThis.AudioContext;
  delete globalThis.AudioWorkletNode;
});

test("a missing chosen microphone falls back to the system default", async () => {
  const requested = [];
  installMedia(async (constraints) => {
    requested.push(constraints.audio.deviceId ?? null);
    if (constraints.audio.deviceId) throw domError("OverconstrainedError");
    return stream();
  });
  const { startDictationCapture } = await import("./audioCapture.ts");
  let fellBack = 0;
  const capture = await startDictationCapture({
    deviceId: "unplugged-headset",
    onDeviceFallback: () => {
      fellBack += 1;
    },
  });
  capture.cancel();
  assert.deepEqual(requested, [{ exact: "unplugged-headset" }, null]);
  assert.equal(fellBack, 1);
});

test("a denied microphone is reported, never retried on another device", async () => {
  let calls = 0;
  installMedia(async () => {
    calls += 1;
    throw domError("NotAllowedError");
  });
  const { startDictationCapture, MicrophoneUnavailableError } = await import(
    "./audioCapture.ts"
  );
  await assert.rejects(
    startDictationCapture({ deviceId: "blackhole" }),
    (error) => error instanceof MicrophoneUnavailableError && error.denied,
  );
  assert.equal(calls, 1);
});

test("the picker lists real inputs only, with a label even before permission", async () => {
  installMedia(
    async () => stream(),
    [
      { kind: "audioinput", deviceId: "default", label: "Default" },
      { kind: "audioinput", deviceId: "mbp", label: "MacBook Pro Microphone" },
      { kind: "audiooutput", deviceId: "spk", label: "Speakers" },
      { kind: "audioinput", deviceId: "bh", label: "" },
    ],
  );
  const { listAudioInputs } = await import("./audioCapture.ts");
  assert.deepEqual(await listAudioInputs(), [
    { deviceId: "mbp", label: "MacBook Pro Microphone" },
    { deviceId: "bh", label: "Microphone 2" },
  ]);
});
