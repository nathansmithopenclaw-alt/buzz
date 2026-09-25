import { pushDictationAudio } from "../api";

/**
 * Microphone capture for dictation.
 *
 * Reuses the huddle's `stt-tap-processor` worklet (48 kHz, 20 ms frames) and
 * forwards 100 ms batches to the backend in order. The mic is released the
 * moment dictation stops or cancels; only then does `stop` wait for batches
 * already in flight, so the system microphone indicator turns off right away.
 */

/** Per-viewer convenience: the chosen dictation microphone ("" = default). */
export const DICTATION_DEVICE_STORAGE_KEY = "buzz-dictation-input-device";

/** Frames per IPC batch: 5 × 20 ms = 100 ms (19.2 KB). */
const FRAMES_PER_BATCH = 5;
/** How long to watch for a dead or muted microphone. */
export const SILENCE_WINDOW_MS = 1500;
/** Peak RMS below which the first window counts as "no audio". */
export const SILENCE_RMS = 0.003;

export type DictationCaptureOptions = {
  deviceId?: string | null;
  /** 0..1 input level, about every 20 ms (for the waveform). */
  onLevel?: (level: number) => void;
  /** Fired once if the first 1.5 s carries no signal at all. */
  onSilence?: () => void;
  /** The chosen device was missing, so the system default was used. */
  onDeviceFallback?: () => void;
};

/** Audio inputs for the microphone picker (labels need mic permission). */
export async function listAudioInputs(): Promise<
  { deviceId: string; label: string }[]
> {
  if (!navigator.mediaDevices?.enumerateDevices) return [];
  const devices = await navigator.mediaDevices.enumerateDevices();
  return devices
    .filter(
      (device) =>
        device.kind === "audioinput" &&
        device.deviceId &&
        device.deviceId !== "default",
    )
    .map((device, index) => ({
      deviceId: device.deviceId,
      label: device.label || `Microphone ${index + 1}`,
    }));
}

export type DictationCaptureResult = {
  /** 100 ms batches the backend refused (the transcript is missing them). */
  droppedBatches: number;
};

export type DictationCapture = {
  /** Releases the mic, then waits for queued audio to reach the backend. */
  stop: () => Promise<DictationCaptureResult>;
  /** Releases the mic without waiting. */
  cancel: () => void;
};

/** Error thrown for a denied or missing microphone. */
export class MicrophoneUnavailableError extends Error {
  readonly denied: boolean;
  constructor(message: string, denied: boolean) {
    super(message);
    this.name = "MicrophoneUnavailableError";
    this.denied = denied;
  }
}

/** RMS of one frame. */
export function frameRms(frame: Float32Array): number {
  if (frame.length === 0) return 0;
  let sum = 0;
  for (let i = 0; i < frame.length; i++) sum += frame[i] * frame[i];
  return Math.sqrt(sum / frame.length);
}

/** Opens the microphone and starts streaming to the active dictation. */
export async function startDictationCapture(
  options: DictationCaptureOptions = {},
): Promise<DictationCapture> {
  if (!navigator.mediaDevices?.getUserMedia) {
    throw new MicrophoneUnavailableError(
      "Microphone capture is not available in this environment.",
      false,
    );
  }
  const open = (deviceId: string | null | undefined) =>
    navigator.mediaDevices.getUserMedia({
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
        sampleRate: 48_000,
        ...(deviceId ? { deviceId: { exact: deviceId } } : {}),
      },
    });
  let stream: MediaStream;
  try {
    try {
      stream = await open(options.deviceId);
    } catch (error) {
      // The chosen microphone is gone (unplugged headset): fall back to the
      // system default rather than leaving dictation unusable.
      const name = (error as DOMException | undefined)?.name;
      const missing =
        name === "OverconstrainedError" || name === "NotFoundError";
      if (!options.deviceId || !missing) throw error;
      stream = await open(null);
      options.onDeviceFallback?.();
    }
  } catch (error) {
    const name = (error as DOMException | undefined)?.name;
    const denied = name === "NotAllowedError" || name === "SecurityError";
    throw new MicrophoneUnavailableError(
      denied
        ? "Allow Buzz to use your microphone for dictation."
        : "Buzz could not open your microphone.",
      denied,
    );
  }

  // Every setup step can throw; each failure releases what was opened so
  // the system microphone indicator never stays on.
  let context: AudioContext | null = null;
  const release = () => {
    for (const track of stream.getTracks()) track.stop();
    void context?.close().catch(() => {});
  };
  let source: MediaStreamAudioSourceNode;
  let node: AudioWorkletNode;
  try {
    context = new AudioContext({ sampleRate: 48_000 });
    if (context.state === "suspended") await context.resume();
    if (context.sampleRate !== 48_000) {
      throw new MicrophoneUnavailableError(
        "Your audio device is not running at 48 kHz.",
        false,
      );
    }
    await context.audioWorklet.addModule("/worklet.js");
    source = context.createMediaStreamSource(stream);
    node = new AudioWorkletNode(context, "stt-tap-processor");
    source.connect(node);
  } catch (error) {
    release();
    throw error instanceof MicrophoneUnavailableError
      ? error
      : new MicrophoneUnavailableError(
          "Buzz could not start dictation audio.",
          false,
        );
  }

  let queue: Promise<unknown> = Promise.resolve();
  let droppedBatches = 0;
  let batch: Float32Array[] = [];
  let released = false;
  const startedAt = performance.now();
  let peak = 0;
  let silenceChecked = false;

  const flush = () => {
    if (batch.length === 0) return;
    const total = batch.reduce((n, f) => n + f.length, 0);
    const joined = new Float32Array(total);
    let offset = 0;
    for (const frame of batch) {
      joined.set(frame, offset);
      offset += frame.length;
    }
    batch = [];
    const bytes = new Uint8Array(joined.buffer);
    queue = queue
      .then(() => pushDictationAudio(bytes))
      .catch(() => {
        droppedBatches += 1;
      });
  };

  node.port.onmessage = (event: MessageEvent<Float32Array>) => {
    if (released) return;
    const frame = event.data;
    const level = frameRms(frame);
    options.onLevel?.(Math.min(1, level * 6));
    if (!silenceChecked) {
      peak = Math.max(peak, level);
      if (performance.now() - startedAt >= SILENCE_WINDOW_MS) {
        silenceChecked = true;
        if (peak < SILENCE_RMS) options.onSilence?.();
      }
    }
    batch.push(frame);
    if (batch.length >= FRAMES_PER_BATCH) flush();
  };

  const releaseMic = () => {
    if (released) return;
    released = true;
    node.port.onmessage = null;
    source.disconnect();
    node.disconnect();
    release();
  };

  return {
    async stop() {
      releaseMic();
      flush();
      await queue;
      return { droppedBatches };
    },
    cancel() {
      batch = [];
      releaseMic();
    },
  };
}
