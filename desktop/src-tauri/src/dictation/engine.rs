//! Local speech-to-text for dictation (Parakeet TDT 0.6B v2 via sherpa-onnx).
//!
//! Mental model:
//!
//! ```text
//! AudioWorklet (48 kHz f32, 20 ms frames)
//!   → dictation_push_audio (raw bytes)
//!   → engine worker thread (owns the recognizer, one session at a time)
//!       rubato 48 kHz → 16 kHz
//!       earshot VAD picks cut points (speech audio is kept whole)
//!       at each ≥500 ms pause, decode the finished chunk while the user talks:
//!       a plain pass and a roster-hotword pass run in parallel, and the
//!       hotword pass's names are spliced into the plain text (`names`)
//!   → dictation_stop: decode the short remainder, return the joined text
//! ```
//!
//! The VAD chooses cut points. Once speech is detected in a chunk every sample
//! is kept; only silence before detected speech is trimmed. If a whole
//! dictation yields no text but carried real signal energy (a speaker too
//! quiet for the detector), the full recording is decoded once as a fallback.
//! The recognizer lives on a dedicated thread because sherpa-onnx objects are
//! CPU-bound and not meant to cross await points.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{mpsc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;

use super::names::{self, RosterNames};

/// earshot requires exactly 256 samples per frame at 16 kHz (16 ms).
const VAD_FRAME_SAMPLES: usize = 256;
/// Probability above which a frame starts speech (earshot operating point
/// shared with the huddle pipeline).
const VAD_ONSET_THRESHOLD: f32 = 0.55;
/// Probability above which a frame counts as voiced once speech has started.
const VAD_OFFSET_THRESHOLD: f32 = 0.35;
/// Consecutive onset frames required before speech is confirmed.
const VAD_ONSET_FRAMES: usize = 3;
/// Trailing silence that ends a chunk (31 frames ≈ 500 ms).
const PAUSE_CUT_FRAMES: usize = 31;
/// Upper bound on one chunk (30 s) so decode latency stays bounded.
const MAX_CHUNK_SAMPLES: usize = 16_000 * 30;
/// Upper bound on one dictation (5 minutes plus a margin for trailing audio).
pub(crate) const MAX_SESSION_SAMPLES_16K: usize = 16_000 * (5 * 60 + 10);
/// Leading silence kept before the first voiced frame of a chunk.
const LEAD_IN_SAMPLES: usize = 16_000 * 3 / 10;
/// Largest raw audio batch accepted from the frontend (100 ms at 48 kHz = 19.2 KB).
pub(crate) const MAX_AUDIO_BATCH_BYTES: usize = 64 * 1024;
/// Hotword boost for roster names. Measured on the dictation holdout: 2.0
/// maximizes name accuracy; 3.0 starts corrupting surrounding words.
const HOTWORDS_SCORE: f32 = 2.0;
/// Beam width for modified beam search (hotwords need beam search).
const MAX_ACTIVE_PATHS: i32 = 4;
/// sherpa-onnx's hotword encoder vocabulary, derived from `tokens.txt`.
const BPE_VOCAB: &str = "bpe.vocab";

// ── Segmentation (pure; unit-tested without a model) ──────────────────────────

/// Splits a 16 kHz stream into decodable chunks at natural pauses.
#[derive(Debug, Default)]
pub(crate) struct Segmenter {
    chunk: Vec<f32>,
    /// Index into `chunk` of the first sample worth decoding (lead-in trimmed).
    chunk_voice_start: Option<usize>,
    onset_frames: usize,
    in_speech: bool,
    voiced_frames: usize,
    trailing_silence: usize,
}

impl Segmenter {
    /// Feeds one 256-sample frame and its speech probability. Returns a chunk
    /// to decode when a pause (or the chunk cap) ends it. Before speech is
    /// detected, only a 300 ms lead-in window is retained; once speech starts,
    /// every sample is kept until the chunk is decoded. Chunks with no
    /// detected speech are not decoded (prevents hallucinated text from room
    /// noise); the session-level fallback covers speakers too quiet to trip
    /// the detector.
    pub(crate) fn push_frame(&mut self, frame: &[f32], probability: f32) -> Option<Vec<f32>> {
        let frame_start = self.chunk.len();
        self.chunk.extend_from_slice(frame);

        if self.in_speech {
            if probability > VAD_OFFSET_THRESHOLD {
                self.voiced_frames += 1;
                self.trailing_silence = 0;
            } else {
                self.trailing_silence += 1;
            }
        } else if probability > VAD_ONSET_THRESHOLD {
            self.onset_frames += 1;
            if self.onset_frames >= VAD_ONSET_FRAMES {
                // Onset frames themselves are speech: the voice window starts
                // at the first of them, minus the lead-in.
                self.in_speech = true;
                let onset_start =
                    frame_start.saturating_sub((VAD_ONSET_FRAMES - 1) * VAD_FRAME_SAMPLES);
                self.chunk_voice_start
                    .get_or_insert(onset_start.saturating_sub(LEAD_IN_SAMPLES));
                self.voiced_frames += self.onset_frames;
                self.onset_frames = 0;
                self.trailing_silence = 0;
            }
        } else {
            self.onset_frames = 0;
            if self.chunk_voice_start.is_none() && self.chunk.len() > LEAD_IN_SAMPLES * 2 {
                // No speech yet in this chunk: keep only the lead-in window.
                let excess = self.chunk.len() - LEAD_IN_SAMPLES;
                self.chunk.drain(..excess);
            }
        }

        let paused = self.in_speech && self.trailing_silence >= PAUSE_CUT_FRAMES;
        if paused || self.chunk.len() >= MAX_CHUNK_SAMPLES {
            return self.take_chunk();
        }
        None
    }

    /// Returns whatever speech remains (used at stop).
    pub(crate) fn finish(&mut self) -> Option<Vec<f32>> {
        self.take_chunk()
    }

    fn take_chunk(&mut self) -> Option<Vec<f32>> {
        let had_voice = self.voiced_frames > 0 || self.chunk_voice_start.is_some();
        let start = self.chunk_voice_start.unwrap_or(0).min(self.chunk.len());
        let chunk = std::mem::take(&mut self.chunk);
        *self = Self::default();
        (had_voice && chunk.len() > start).then(|| chunk[start..].to_vec())
    }
}

// ── Audio helpers ─────────────────────────────────────────────────────────────

/// Raw little-endian f32 bytes → samples (extra bytes ignored).
pub(crate) fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

struct Resampler48To16 {
    inner: rubato::Fft<f32>,
    chunk_in: usize,
    pending: Vec<f32>,
}

impl Resampler48To16 {
    fn new() -> Result<Self, String> {
        use rubato::{FixedSync, Resampler};
        let inner = rubato::Fft::<f32>::new(48_000, 16_000, 1024, 2, 1, FixedSync::Input)
            .map_err(|error| format!("dictation resampler init failed: {error}"))?;
        let chunk_in = inner.input_frames_next();
        Ok(Self {
            inner,
            chunk_in,
            pending: Vec::with_capacity(chunk_in * 2),
        })
    }

    fn push(&mut self, samples_48k: &[f32], out: &mut Vec<f32>) {
        self.pending.extend_from_slice(samples_48k);
        while self.pending.len() >= self.chunk_in {
            let chunk: Vec<f32> = self.pending.drain(..self.chunk_in).collect();
            out.extend(self.process(&chunk));
        }
    }

    /// Pads the remainder with silence so the last words reach the decoder.
    fn flush(&mut self, out: &mut Vec<f32>) {
        if self.pending.is_empty() {
            return;
        }
        let mut chunk = std::mem::take(&mut self.pending);
        chunk.resize(self.chunk_in, 0.0);
        out.extend(self.process(&chunk));
    }

    fn process(&mut self, chunk_48k: &[f32]) -> Vec<f32> {
        use audioadapter_buffers::direct::InterleavedSlice;
        use rubato::Resampler;
        let Ok(input) = InterleavedSlice::new(chunk_48k, 1, chunk_48k.len()) else {
            return Vec::new();
        };
        self.inner
            .process(&input, 0, None)
            .map(|out| out.take_data())
            .unwrap_or_default()
    }
}

// ── Recognizer ────────────────────────────────────────────────────────────────

/// Intra-op threads for the dictation decoder. Dictation decodes while the
/// user is not speaking into a huddle, so it can use more than the huddle's 1.
fn decoder_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get().min(4) as i32)
        .unwrap_or(2)
        .max(1)
}

/// A loaded model plus the characters its vocabulary can spell.
struct Recognizer {
    inner: sherpa_onnx::OfflineRecognizer,
    encodable: HashSet<char>,
}

/// Writes `bpe.vocab` ("piece\tscore" per line) next to `tokens.txt`.
/// Without `modeling_unit = "bpe"` and this file, sherpa-onnx falls back to
/// CJK characters and silently drops every English hotword.
fn prepare_bpe_vocab(model_dir: &Path, tokens: &str) -> Result<PathBuf, String> {
    let vocab: String = tokens
        .lines()
        .filter_map(|line| line.rsplit_once(' ').map(|(piece, _)| piece))
        .map(|piece| format!("{piece}\t-1.0\n"))
        .collect();
    let path = model_dir.join(BPE_VOCAB);
    if std::fs::read_to_string(&path).is_ok_and(|existing| existing == vocab) {
        return Ok(path);
    }
    let staged = model_dir.join(format!("{BPE_VOCAB}.tmp"));
    std::fs::write(&staged, vocab)
        .and_then(|()| std::fs::rename(&staged, &path))
        .map_err(|e| format!("could not prepare the dictation vocabulary: {e}"))?;
    Ok(path)
}

fn load_recognizer(model_dir: &Path) -> Result<Recognizer, String> {
    use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};
    let file = |name: &str| -> Result<String, String> {
        let path = model_dir.join(name);
        if path.is_file() {
            Ok(path.to_string_lossy().into_owned())
        } else {
            Err(format!("dictation model file missing: {name}"))
        }
    };
    let tokens_path = file("tokens.txt")?;
    let tokens = std::fs::read_to_string(&tokens_path)
        .map_err(|e| format!("could not read the dictation vocabulary: {e}"))?;
    let encodable = tokens
        .lines()
        .filter_map(|line| line.rsplit_once(' ').map(|(piece, _)| piece))
        .flat_map(|piece| piece.chars().filter(|&c| c != '\u{2581}'))
        .collect();
    let bpe_vocab = prepare_bpe_vocab(model_dir, &tokens)?;
    let mut cfg = OfflineRecognizerConfig::default();
    cfg.model_config.transducer.encoder = Some(file("encoder.int8.onnx")?);
    cfg.model_config.transducer.decoder = Some(file("decoder.int8.onnx")?);
    cfg.model_config.transducer.joiner = Some(file("joiner.int8.onnx")?);
    cfg.model_config.tokens = Some(tokens_path);
    cfg.model_config.model_type = Some("nemo_transducer".to_string());
    cfg.model_config.modeling_unit = Some("bpe".to_string());
    cfg.model_config.bpe_vocab = Some(bpe_vocab.to_string_lossy().into_owned());
    cfg.model_config.num_threads = decoder_threads();
    cfg.model_config.debug = false;
    cfg.decoding_method = Some("modified_beam_search".to_string());
    cfg.max_active_paths = MAX_ACTIVE_PATHS;
    cfg.hotwords_score = HOTWORDS_SCORE;
    let inner = OfflineRecognizer::create(&cfg)
        .ok_or_else(|| "could not load the dictation speech model".to_string())?;
    Ok(Recognizer { inner, encodable })
}

fn decode(recognizer: &Recognizer, samples_16k: &[f32], hotwords: Option<&str>) -> String {
    let stream = match hotwords {
        Some(hotwords) => recognizer.inner.create_stream_with_hotwords(hotwords),
        None => recognizer.inner.create_stream(),
    };
    stream.accept_waveform(16_000, samples_16k);
    recognizer.inner.decode(&stream);
    stream
        .get_result()
        .map(|r| r.text.trim().to_string())
        .unwrap_or_default()
}

/// Decodes a chunk plainly and, when the roster has names, with hotwords in
/// parallel; returns (plain, hotword) text.
fn decode_both(
    recognizer: &Recognizer,
    samples_16k: &[f32],
    names: &RosterNames,
) -> (String, String) {
    let Some(hotwords) = names.hotwords() else {
        return (decode(recognizer, samples_16k, None), String::new());
    };
    thread::scope(|scope| {
        let hot = scope.spawn(|| decode(recognizer, samples_16k, Some(hotwords)));
        let plain = decode(recognizer, samples_16k, None);
        (plain, hot.join().unwrap_or_default())
    })
}

// ── Session ───────────────────────────────────────────────────────────────────

/// Timing for one finished dictation, reported to the frontend and samples.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Transcript {
    pub text: String,
    pub audio_ms: u64,
    /// Total decode time spent across all chunks.
    pub decode_ms: u64,
    /// Decode time spent after stop (the user-visible wait).
    pub tail_decode_ms: u64,
    pub chunks: u32,
    /// Set when this dictation was written to the local capture directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_id: Option<String>,
    /// Audio and per-chunk decodes, kept only when the session was started
    /// with capture on (see `capture`).
    #[serde(skip)]
    pub capture: Option<CaptureData>,
}

/// One decoded chunk: both passes and the spliced result.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChunkDecode {
    pub plain: String,
    pub hotword: String,
    pub text: String,
    pub decode_ms: u64,
}

/// What a capture needs from the engine.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CaptureData {
    pub audio_16k: Vec<f32>,
    pub chunks: Vec<ChunkDecode>,
    pub hotword_names: usize,
}

struct Session {
    id: u64,
    names: RosterNames,
    /// Keep audio and chunk decodes for a capture.
    capture: bool,
    decodes: Vec<ChunkDecode>,
    resampler: Resampler48To16,
    vad: earshot::Detector<earshot::DefaultPredictor>,
    segmenter: Segmenter,
    frame_carry: Vec<f32>,
    /// Every 16 kHz sample of the session, for the quiet-speaker fallback.
    all_audio: Vec<f32>,
    texts: Vec<String>,
    samples_16k: usize,
    decode_ms: u64,
    chunks: u32,
}

impl Session {
    fn new(id: u64, names: RosterNames, capture: bool) -> Result<Self, String> {
        Ok(Self {
            id,
            names,
            capture,
            decodes: Vec::new(),
            resampler: Resampler48To16::new()?,
            vad: earshot::Detector::new(earshot::DefaultPredictor::new()),
            segmenter: Segmenter::default(),
            frame_carry: Vec::new(),
            all_audio: Vec::new(),
            texts: Vec::new(),
            samples_16k: 0,
            decode_ms: 0,
            chunks: 0,
        })
    }

    fn push_16k(&mut self, samples: &[f32], recognizer: &Recognizer) {
        let room = MAX_SESSION_SAMPLES_16K.saturating_sub(self.samples_16k);
        let samples = &samples[..samples.len().min(room)];
        self.samples_16k += samples.len();
        self.all_audio.extend_from_slice(samples);
        self.frame_carry.extend_from_slice(samples);
        while self.frame_carry.len() >= VAD_FRAME_SAMPLES {
            let frame: Vec<f32> = self.frame_carry.drain(..VAD_FRAME_SAMPLES).collect();
            let clamped: Vec<f32> = frame.iter().map(|s| s.clamp(-1.0, 1.0)).collect();
            let probability = self.vad.predict_f32(&clamped);
            if let Some(chunk) = self.segmenter.push_frame(&frame, probability) {
                self.decode_chunk(&chunk, recognizer);
            }
        }
    }

    fn push_48k(&mut self, samples_48k: &[f32], recognizer: &Recognizer) {
        let mut out = Vec::new();
        self.resampler.push(samples_48k, &mut out);
        self.push_16k(&out, recognizer);
    }

    fn decode_chunk(&mut self, chunk: &[f32], recognizer: &Recognizer) -> u64 {
        let started = Instant::now();
        let (plain, hotword) = decode_both(recognizer, chunk, &self.names);
        let text = if hotword.is_empty() {
            plain.clone()
        } else {
            names::splice_names(&plain, &hotword, &self.names)
        };
        let elapsed = started.elapsed().as_millis() as u64;
        self.decode_ms += elapsed;
        self.chunks += 1;
        if self.capture {
            self.decodes.push(ChunkDecode {
                plain,
                hotword,
                text: text.clone(),
                decode_ms: elapsed,
            });
        }
        if !text.is_empty() {
            self.texts.push(text);
        }
        elapsed
    }

    fn finish(mut self, recognizer: &Recognizer) -> Transcript {
        let mut tail = Vec::new();
        self.resampler.flush(&mut tail);
        self.push_16k(&tail, recognizer);
        // Remaining partial frame joins the final chunk.
        let carry = std::mem::take(&mut self.frame_carry);
        let mut tail_decode_ms = 0;
        let mut last = self.segmenter.finish().unwrap_or_default();
        if !last.is_empty() || !carry.is_empty() {
            last.extend_from_slice(&carry);
            if last.len() > carry.len() {
                tail_decode_ms = self.decode_chunk(&last, recognizer);
            }
        }
        if self.texts.is_empty() && has_signal(&self.all_audio) {
            let audio = self.all_audio.clone();
            tail_decode_ms += self.decode_chunk(&audio, recognizer);
        }
        let capture = self.capture.then(|| CaptureData {
            audio_16k: std::mem::take(&mut self.all_audio),
            chunks: std::mem::take(&mut self.decodes),
            hotword_names: self.names.hotwords().map_or(0, |h| h.split('/').count()),
        });
        Transcript {
            text: join_chunks(&self.texts),
            audio_ms: (self.samples_16k as u64 * 1000) / 16_000,
            decode_ms: self.decode_ms,
            tail_decode_ms,
            chunks: self.chunks,
            capture_id: None,
            capture,
        }
    }
}

/// RMS above which a recording is treated as containing real sound
/// (≈ -40 dBFS; a muted or disconnected mic sits far below).
const SIGNAL_RMS_THRESHOLD: f32 = 0.01;

/// `true` when the audio carries more than background silence.
pub(crate) fn has_signal(samples: &[f32]) -> bool {
    if samples.is_empty() {
        return false;
    }
    let energy: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    ((energy / samples.len() as f64).sqrt() as f32) > SIGNAL_RMS_THRESHOLD
}

/// Joins decoded chunks into one transcript with single spaces. Each chunk is
/// decoded without the previous one's context, so a chunk that follows a
/// finished sentence gets its first letter capitalized.
pub(crate) fn join_chunks(texts: &[String]) -> String {
    let mut out = String::new();
    for text in texts {
        let chunk = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if chunk.is_empty() {
            continue;
        }
        if out.is_empty() {
            out.push_str(&chunk);
            continue;
        }
        let sentence_ended = out.ends_with(['.', '!', '?']);
        out.push(' ');
        let mut chars = chunk.chars();
        match chars.next() {
            Some(first) if sentence_ended && first.is_lowercase() => {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
            _ => out.push_str(&chunk),
        }
    }
    out
}

// ── Worker thread ─────────────────────────────────────────────────────────────

enum Command {
    Start {
        id: u64,
        roster: Vec<String>,
        capture: bool,
        reply: mpsc::Sender<Result<(), String>>,
    },
    Audio {
        id: u64,
        bytes: Vec<u8>,
    },
    Stop {
        id: u64,
        reply: mpsc::Sender<Result<Transcript, String>>,
    },
    Cancel {
        id: u64,
    },
}

struct Worker {
    tx: mpsc::Sender<Command>,
    model_dir: PathBuf,
}

fn worker_slot() -> &'static Mutex<Option<Worker>> {
    static WORKER: OnceLock<Mutex<Option<Worker>>> = OnceLock::new();
    WORKER.get_or_init(|| Mutex::new(None))
}

fn spawn_worker(model_dir: PathBuf) -> Result<Worker, String> {
    let (tx, rx) = mpsc::channel::<Command>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let dir = model_dir.clone();
    thread::Builder::new()
        .name("dictation-stt".into())
        .spawn(move || {
            let recognizer = match load_recognizer(&dir) {
                Ok(recognizer) => {
                    let _ = ready_tx.send(Ok(()));
                    recognizer
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            run_worker(rx, &recognizer);
        })
        .map_err(|e| format!("could not start the dictation worker: {e}"))?;
    ready_rx
        .recv_timeout(Duration::from_secs(60))
        .map_err(|_| "the dictation speech model did not load in time".to_string())??;
    Ok(Worker { tx, model_dir })
}

fn run_worker(rx: mpsc::Receiver<Command>, recognizer: &Recognizer) {
    let mut session: Option<Session> = None;
    while let Ok(command) = rx.recv() {
        match command {
            Command::Start {
                id,
                roster,
                capture,
                reply,
            } => {
                let names = RosterNames::new(&roster, &recognizer.encodable);
                let result = Session::new(id, names, capture).map(|fresh| {
                    session = Some(fresh);
                });
                let _ = reply.send(result);
            }
            Command::Audio { id, bytes } => {
                if let Some(active) = session.as_mut().filter(|s| s.id == id) {
                    active.push_48k(&bytes_to_f32(&bytes), recognizer);
                }
            }
            Command::Stop { id, reply } => {
                let result = match session.take() {
                    Some(active) if active.id == id => Ok(active.finish(recognizer)),
                    other => {
                        session = other;
                        Err("no matching dictation is recording".to_string())
                    }
                };
                let _ = reply.send(result);
            }
            Command::Cancel { id } => {
                if session.as_ref().is_some_and(|s| s.id == id) {
                    session = None;
                }
            }
        }
    }
}

/// Sender for the running worker, starting (and loading the model) on first use.
fn worker_sender(model_dir: &Path) -> Result<mpsc::Sender<Command>, String> {
    let mut slot = worker_slot().lock().map_err(|e| e.to_string())?;
    if let Some(worker) = slot.as_ref().filter(|w| w.model_dir == model_dir) {
        return Ok(worker.tx.clone());
    }
    let worker = spawn_worker(model_dir.to_path_buf())?;
    let tx = worker.tx.clone();
    *slot = Some(worker);
    Ok(tx)
}

fn existing_sender() -> Option<mpsc::Sender<Command>> {
    worker_slot()
        .lock()
        .ok()?
        .as_ref()
        .map(|worker| worker.tx.clone())
}

/// Starts a session biased toward `roster` names; blocks while the model
/// loads on first use (~1 s). `capture` keeps the audio for a local capture.
pub(crate) fn start(
    model_dir: &Path,
    id: u64,
    roster: Vec<String>,
    capture: bool,
) -> Result<(), String> {
    let tx = worker_sender(model_dir)?;
    let (reply, rx) = mpsc::channel();
    tx.send(Command::Start {
        id,
        roster,
        capture,
        reply,
    })
    .map_err(|_| "the dictation worker stopped".to_string())?;
    rx.recv_timeout(Duration::from_secs(10))
        .map_err(|_| "the dictation worker did not respond".to_string())?
}

/// Queues audio for the session; never blocks the IPC thread.
pub(crate) fn push_audio(id: u64, bytes: Vec<u8>) {
    if let Some(tx) = existing_sender() {
        let _ = tx.send(Command::Audio { id, bytes });
    }
}

/// Finishes the session and returns the transcript (blocking; call from a
/// blocking task).
pub(crate) fn stop(id: u64) -> Result<Transcript, String> {
    let tx = existing_sender().ok_or_else(|| "dictation is not running".to_string())?;
    let (reply, rx) = mpsc::channel();
    tx.send(Command::Stop { id, reply })
        .map_err(|_| "the dictation worker stopped".to_string())?;
    rx.recv_timeout(Duration::from_secs(60))
        .map_err(|_| "the dictation worker did not finish in time".to_string())?
}

pub(crate) fn cancel(id: u64) {
    if let Some(tx) = existing_sender() {
        let _ = tx.send(Command::Cancel { id });
    }
}

/// Decodes a whole 16 kHz buffer through the same segmentation as live
/// dictation. Used by the eval test and the setup page self-test.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn transcribe_16k_blocking(
    model_dir: &Path,
    samples_16k: &[f32],
    roster: &[String],
) -> Result<Transcript, String> {
    let recognizer = load_recognizer(model_dir)?;
    let names = RosterNames::new(roster, &recognizer.encodable);
    let mut session = Session::new(0, names, false)?;
    // Feed in 20 ms batches to mirror the live cadence.
    for batch in samples_16k.chunks(320) {
        session.push_16k(batch, &recognizer);
    }
    Ok(session.finish(&recognizer))
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
