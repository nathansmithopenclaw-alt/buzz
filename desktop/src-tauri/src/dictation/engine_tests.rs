use super::*;

const SILENT: f32 = 0.05;
const SPEECH: f32 = 0.9;

fn frame(value: f32) -> Vec<f32> {
    vec![value; VAD_FRAME_SAMPLES]
}

/// Feeds `count` frames at `probability`; returns every chunk emitted.
fn feed(seg: &mut Segmenter, count: usize, probability: f32, value: f32) -> Vec<Vec<f32>> {
    (0..count)
        .filter_map(|_| seg.push_frame(&frame(value), probability))
        .collect()
}

#[test]
fn a_pause_cuts_a_chunk_with_lead_in_and_trailing_pause() {
    let mut seg = Segmenter::default();
    assert!(
        feed(&mut seg, 40, SILENT, 0.0).is_empty(),
        "silence alone never cuts"
    );
    assert!(
        feed(&mut seg, 50, SPEECH, 0.5).is_empty(),
        "speech alone never cuts early"
    );
    let chunks = feed(&mut seg, PAUSE_CUT_FRAMES, SILENT, 0.0);
    assert_eq!(
        chunks.len(),
        1,
        "a ≥500 ms pause after speech cuts exactly once"
    );
    let chunk = &chunks[0];
    let speech_samples = chunk.iter().filter(|&&s| s == 0.5).count();
    assert_eq!(
        speech_samples,
        50 * VAD_FRAME_SAMPLES,
        "every speech sample is kept"
    );
    let lead_in = chunk.iter().take_while(|&&s| s == 0.0).count();
    assert_eq!(
        lead_in, LEAD_IN_SAMPLES,
        "300 ms of lead-in precedes the speech"
    );
}

#[test]
fn onset_frames_count_as_speech_not_silence() {
    let mut seg = Segmenter::default();
    feed(&mut seg, VAD_ONSET_FRAMES, SPEECH, 0.5);
    assert!(seg.in_speech);
    assert_eq!(seg.trailing_silence, 0);
    assert_eq!(seg.voiced_frames, VAD_ONSET_FRAMES);
    // One short of the cut: still one chunk in progress.
    assert!(feed(&mut seg, PAUSE_CUT_FRAMES - 1, SILENT, 0.0).is_empty());
    assert_eq!(feed(&mut seg, 1, SILENT, 0.0).len(), 1);
}

#[test]
fn short_pauses_inside_speech_do_not_split() {
    let mut seg = Segmenter::default();
    feed(&mut seg, 20, SPEECH, 0.5);
    assert!(feed(&mut seg, PAUSE_CUT_FRAMES - 5, SILENT, 0.0).is_empty());
    feed(&mut seg, 20, SPEECH, 0.5);
    let finished = seg.finish().expect("speech remains");
    assert_eq!(
        finished.iter().filter(|&&s| s == 0.5).count(),
        40 * VAD_FRAME_SAMPLES,
        "both phrases stay in one chunk"
    );
}

#[test]
fn pure_silence_is_never_decoded() {
    let mut seg = Segmenter::default();
    assert!(feed(&mut seg, 500, SILENT, 0.0).is_empty());
    assert!(
        seg.chunk.len() <= LEAD_IN_SAMPLES * 2,
        "silence is trimmed, not accumulated"
    );
    assert_eq!(seg.finish(), None);
}

#[test]
fn brief_blips_below_onset_count_do_not_start_speech() {
    let mut seg = Segmenter::default();
    feed(&mut seg, VAD_ONSET_FRAMES - 1, SPEECH, 0.5);
    feed(&mut seg, 1, SILENT, 0.0);
    assert!(!seg.in_speech);
    assert_eq!(seg.finish(), None);
}

#[test]
fn long_speech_is_capped_at_thirty_seconds_per_chunk() {
    let mut seg = Segmenter::default();
    let frames = MAX_CHUNK_SAMPLES / VAD_FRAME_SAMPLES + 10;
    let chunks = feed(&mut seg, frames, SPEECH, 0.5);
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].len() <= MAX_CHUNK_SAMPLES);
    assert!(
        seg.finish().is_some(),
        "speech after the cap continues in a new chunk"
    );
}

#[test]
fn signal_detection_separates_silence_from_sound() {
    assert!(!has_signal(&[]));
    assert!(!has_signal(&vec![0.0; 16_000]));
    assert!(!has_signal(&vec![0.001; 16_000]));
    assert!(has_signal(&vec![0.1; 16_000]));
}

#[test]
fn bytes_convert_little_endian_and_ignore_partial_words() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0.5f32.to_le_bytes());
    bytes.extend_from_slice(&(-1.0f32).to_le_bytes());
    bytes.push(0xFF);
    assert_eq!(bytes_to_f32(&bytes), vec![0.5, -1.0]);
}

#[test]
fn chunks_join_with_single_spaces() {
    let texts = vec![
        "Anvil, can you review".to_string(),
        "  ".to_string(),
        "the patch  Codex sent?".to_string(),
    ];
    assert_eq!(
        join_chunks(&texts),
        "Anvil, can you review the patch Codex sent?"
    );
}

#[test]
fn chunks_after_a_finished_sentence_start_capitalized() {
    let texts = vec![
        "Let's start with the overall appearance.".to_string(),
        "isn't really matching what we settled on".to_string(),
        "because the table is tilted".to_string(),
    ];
    assert_eq!(
        join_chunks(&texts),
        "Let's start with the overall appearance. Isn't really matching what we settled on because the table is tilted"
    );
}

// ── Model eval (opt-in) ───────────────────────────────────────────────────────

fn read_wav_mono_16k(path: &Path) -> Vec<f32> {
    // Minimal RIFF walk: PCM16 mono 16 kHz (the eval harness format).
    let bytes = std::fs::read(path).expect("read wav");
    let mut offset = 12;
    let mut data = None;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        if id == b"data" {
            data = Some(&bytes[offset + 8..(offset + 8 + size).min(bytes.len())]);
            break;
        }
        offset += 8 + size + (size & 1);
    }
    data.expect("data chunk")
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect()
}

/// Runs the real Parakeet 0.6B v2 model over the dictation eval clips through
/// the live segmentation path. Prints one line per clip for the harness to
/// score against `cases.json`.
///
///   CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true \
///   BUZZ_DICTATION_MODEL_DIR=~/.buzz/models/parakeet-tdt-0.6b-v2-en \
///   BUZZ_DICTATION_EVAL_AUDIO=/path/to/audio \
///   cargo test --release --lib dictation::engine::tests::eval -- --ignored --nocapture
///
/// (Debug assertions stay on because some crate test modules only compile
/// with them; release optimisation keeps decode timing realistic.)
#[test]
#[ignore = "needs the dictation model and eval audio on disk"]
fn eval_clips_through_live_segmentation() {
    let model_dir = PathBuf::from(std::env::var("BUZZ_DICTATION_MODEL_DIR").expect("model dir"));
    let audio_dir = PathBuf::from(std::env::var("BUZZ_DICTATION_EVAL_AUDIO").expect("audio dir"));
    let mut paths: Vec<_> = std::fs::read_dir(&audio_dir)
        .expect("audio dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    paths.sort();
    for path in paths {
        let samples = read_wav_mono_16k(&path);
        let started = Instant::now();
        let transcript = transcribe_16k_blocking(&model_dir, &samples, &[]).expect("transcribe");
        println!(
            "EVAL\t{}\t{}\t{}\t{}",
            path.file_stem().unwrap().to_string_lossy(),
            started.elapsed().as_millis(),
            transcript.chunks,
            transcript.text
        );
    }
}
