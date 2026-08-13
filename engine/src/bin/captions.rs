//! Captions subsystem — whisper.cpp CLI/model discovery, JSON segment
//! parsing, and the worker-thread transcription job.
//!
//! Extracted from `preview.rs` (dualcut#78): `find_whisper` /
//! `find_whisper_model` resolve the CLI + model, `parse_whisper_segments`
//! is the pure JSON → (start, end, text) parser (unit-tested here), and
//! `run_captions_job` drives the subprocess. The GTK dialog and editor
//! mutation (`apply_captions` / `apply_karaoke_captions` /
//! `show_captions_dialog`) stay in `preview.rs`.

use super::*;

/// The bundled whisper-cli binary shipped by the Flatpak (`whisper-cpp`
/// module in the manifest), checked before falling back to PATH.
pub(crate) const BUNDLED_WHISPER_CLI: &str = "/app/bin/whisper-cli";

/// The bundled ggml model shipped by the Flatpak (`whisper-model`
/// module in the manifest), used when `DUALCUT_WHISPER_MODEL` is unset.
pub(crate) const BUNDLED_WHISPER_MODEL: &str = "/app/share/dualcut/models/ggml-tiny.en-q5_1.bin";

/// Locate a whisper.cpp CLI: the Flatpak-bundled `/app/bin/whisper-cli`
/// first (so captions work out of the box in the Flatpak build), then
/// PATH (#37): `whisper-cli` (current name), then the older
/// `whisper-cpp`, for users with their own install.
pub(crate) fn find_whisper() -> Option<PathBuf> {
    let bundled = PathBuf::from(BUNDLED_WHISPER_CLI);
    if bundled.is_file() {
        return Some(bundled);
    }
    let path = std::env::var_os("PATH")?;
    for name in ["whisper-cli", "whisper-cpp"] {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Resolve the whisper model path: `DUALCUT_WHISPER_MODEL` if set,
/// otherwise the Flatpak-bundled model if present on disk.
pub(crate) fn find_whisper_model() -> Option<String> {
    if let Some(m) = std::env::var("DUALCUT_WHISPER_MODEL")
        .ok()
        .filter(|m| !m.trim().is_empty())
    {
        return Some(m);
    }
    let bundled = PathBuf::from(BUNDLED_WHISPER_MODEL);
    if bundled.is_file() {
        return Some(bundled.to_string_lossy().into_owned());
    }
    None
}

/// Parse whisper.cpp `--output-json` into (start, end, text) seconds.
/// Shape (docs/recipes/auto-captions.md): `transcription[].offsets.{from,to}`
/// in milliseconds plus `text`.
pub(crate) fn parse_whisper_segments(json: &str) -> std::result::Result<Vec<(f64, f64, String)>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("bad whisper JSON: {e}"))?;
    let segments = value
        .get("transcription")
        .and_then(|t| t.as_array())
        .ok_or_else(|| "whisper JSON has no transcription array".to_string())?;
    Ok(segments
        .iter()
        .filter_map(|seg| {
            let offsets = seg.get("offsets")?;
            let from = offsets.get("from")?.as_f64()? / 1000.0;
            let to = offsets.get("to")?.as_f64()? / 1000.0;
            let text = seg.get("text")?.as_str()?.to_string();
            Some((from, to, text))
        })
        .collect())
}

/// Worker-thread half of auto-captions (#37): export the project audio to
/// a temp wav, transcribe it with whisper.cpp, parse the segments.
/// `word_level` (#47) requests whisper.cpp segment the transcript down to
/// ~single words (`--max-len 1`) instead of whole phrases -- same JSON
/// shape, so `parse_whisper_segments` needs no changes either way.
pub(crate) fn run_captions_job(
    project_json: String,
    base_dir: PathBuf,
    whisper: PathBuf,
    model: String,
    word_level: bool,
) -> std::result::Result<Vec<(f64, f64, String)>, String> {
    let tmp = std::env::temp_dir().join(format!("dualcut-captions-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| format!("temp dir: {e}"))?;
    let wav = tmp.join("voice.wav");
    dualcut_engine::render_project(&project_json, &base_dir, &wav.to_string_lossy(), "wav")
        .map_err(|e| format!("audio export failed: {e:#}"))?;
    let prefix = tmp.join("voice");
    let mut cmd = std::process::Command::new(&whisper);
    cmd.arg("-m").arg(&model).arg("-f").arg(&wav).arg("--output-json").arg("--output-file").arg(&prefix);
    if word_level {
        cmd.arg("--max-len").arg("1");
    }
    let output = cmd.output().map_err(|e| format!("running {}: {e}", whisper.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "whisper failed: {}",
            stderr.lines().last().unwrap_or("unknown error")
        ));
    }
    let json = std::fs::read_to_string(prefix.with_extension("json"))
        .map_err(|e| format!("reading whisper output: {e}"))?;
    parse_whisper_segments(&json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_whisper_segments_extracts_seconds_and_text() {
        let json = r#"{"transcription":[
            {"offsets":{"from":0,"to":1500},"text":" Hello"},
            {"offsets":{"from":1500,"to":3000},"text":" world"}
        ]}"#;
        let segments = parse_whisper_segments(json).expect("valid whisper JSON parses");
        assert_eq!(segments, vec![(0.0, 1.5, " Hello".to_string()), (1.5, 3.0, " world".to_string())]);
    }

    #[test]
    fn parse_whisper_segments_rejects_malformed_json() {
        assert!(parse_whisper_segments("not json").is_err());
    }

    #[test]
    fn parse_whisper_segments_rejects_json_missing_the_transcription_array() {
        assert!(parse_whisper_segments(r#"{"foo":"bar"}"#).is_err());
    }

    #[test]
    fn parse_whisper_segments_skips_segments_missing_required_fields() {
        let json = r#"{"transcription":[{"text":"no offsets"}]}"#;
        let segments = parse_whisper_segments(json).expect("still parses, just yields nothing");
        assert!(segments.is_empty());
    }
}
