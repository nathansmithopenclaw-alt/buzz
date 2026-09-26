//! Local dictation captures, for evaluating recognition on the user's own
//! voice. Off unless `<app data>/dictation-captures/` exists; captures never
//! leave the machine.
//!
//! Each dictation writes `<id>.wav` (16 kHz mono PCM) and `<id>.json` (both
//! decoding passes per chunk and the spliced transcript). The composer then
//! appends notes to the JSON: what it inserted, the polish result, and the
//! message as sent.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use super::engine::{CaptureData, Transcript};

const DIR: &str = "dictation-captures";
/// Oldest captures are pruned past this many (a 5-minute clip is ~10 MB).
const MAX_CAPTURES: usize = 2000;
const MAX_NOTE_BYTES: usize = 64 * 1024;

/// The capture directory, when the user has opted in by creating it.
pub(crate) fn dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?.join(DIR);
    dir.is_dir().then_some(dir)
}

fn wav_bytes(samples_16k: &[f32]) -> Vec<u8> {
    let data_len = (samples_16k.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&16_000u32.to_le_bytes());
    out.extend_from_slice(&32_000u32.to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples_16k {
        let pcm = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        out.extend_from_slice(&pcm.to_le_bytes());
    }
    out
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Writes one capture and returns its id.
pub(crate) fn write(
    dir: &Path,
    session_id: u64,
    transcript: &Transcript,
    data: &CaptureData,
) -> Result<String, String> {
    let now = chrono::Local::now();
    let id = format!("{}-{session_id}", now.format("%Y%m%d-%H%M%S"));
    let record = json!({
        "id": id,
        "createdAt": now.to_rfc3339(),
        "text": transcript.text,
        "audioMs": transcript.audio_ms,
        "decodeMs": transcript.decode_ms,
        "tailDecodeMs": transcript.tail_decode_ms,
        "hotwordNames": data.hotword_names,
        "chunks": data.chunks,
        "notes": [],
    });
    std::fs::write(dir.join(format!("{id}.wav")), wav_bytes(&data.audio_16k))
        .and_then(|()| {
            std::fs::write(
                dir.join(format!("{id}.json")),
                serde_json::to_vec_pretty(&record).unwrap_or_default(),
            )
        })
        .map_err(|e| format!("could not write the dictation capture: {e}"))?;
    prune(dir);
    Ok(id)
}

/// Appends a note (`{"kind": ..., ...}`) to a capture's record.
pub(crate) fn note(dir: &Path, id: &str, note: Value) -> Result<(), String> {
    if !valid_id(id) {
        return Err("invalid capture id".into());
    }
    if serde_json::to_vec(&note).map_or(true, |bytes| bytes.len() > MAX_NOTE_BYTES) {
        return Err("capture note too large".into());
    }
    let path = dir.join(format!("{id}.json"));
    let mut record: Value = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("capture not found")?;
    let entry = json!({ "at": chrono::Local::now().to_rfc3339(), "note": note });
    match record.get_mut("notes").and_then(Value::as_array_mut) {
        Some(notes) => notes.push(entry),
        None => record["notes"] = json!([entry]),
    }
    let staged = path.with_extension("json.tmp");
    std::fs::write(
        &staged,
        serde_json::to_vec_pretty(&record).unwrap_or_default(),
    )
    .and_then(|()| std::fs::rename(&staged, &path))
    .map_err(|e| format!("could not update the dictation capture: {e}"))
}

/// Keeps the newest `MAX_CAPTURES` captures.
fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut ids: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json").map(str::to_string)
        })
        .filter(|id| valid_id(id))
        .collect();
    if ids.len() <= MAX_CAPTURES {
        return;
    }
    // Ids start with a sortable timestamp.
    ids.sort();
    for id in &ids[..ids.len() - MAX_CAPTURES] {
        let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
        let _ = std::fs::remove_file(dir.join(format!("{id}.wav")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript() -> (Transcript, CaptureData) {
        let data = CaptureData {
            audio_16k: vec![0.0, 0.5, -1.0, 2.0],
            chunks: Vec::new(),
            hotword_names: 3,
        };
        let transcript = Transcript {
            text: "Bolt, ship it.".into(),
            audio_ms: 1000,
            decode_ms: 80,
            tail_decode_ms: 40,
            chunks: 1,
            capture_id: None,
            capture: None,
        };
        (transcript, data)
    }

    #[test]
    fn writes_a_wav_and_a_record_and_appends_notes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (transcript, data) = transcript();
        let id = write(dir.path(), 7, &transcript, &data).expect("write");
        let wav = std::fs::read(dir.path().join(format!("{id}.wav"))).expect("wav");
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(wav.len(), 44 + 4 * 2);
        // Clamped: 2.0 is written as full scale, like 1.0.
        assert_eq!(i16::from_le_bytes([wav[50], wav[51]]), i16::MAX);
        note(dir.path(), &id, json!({"kind": "sent", "content": "hi"})).expect("note");
        let record: Value = serde_json::from_slice(
            &std::fs::read(dir.path().join(format!("{id}.json"))).expect("json"),
        )
        .expect("parse");
        assert_eq!(record["text"], "Bolt, ship it.");
        assert_eq!(record["notes"][0]["note"]["kind"], "sent");
    }

    #[test]
    fn notes_refuse_ids_that_could_leave_the_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(note(dir.path(), "../settings", json!({})).is_err());
        assert!(note(dir.path(), "", json!({})).is_err());
    }
}
