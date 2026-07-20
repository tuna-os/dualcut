//! Silence detection for the "remove silences" op (#46). Pure, testable
//! logic here; PCM decoding (which needs GStreamer, feature = "preview")
//! lives in `thumbs::decode_mono_pcm`.

/// Detect silent stretches in a mono PCM stream. `samples` are normalized
/// float samples; `sample_rate` in Hz. A stretch is "silent" when its
/// windowed RMS stays below `threshold_db` (dBFS, e.g. -40.0) for at least
/// `min_duration` seconds. Returns (start, end) in seconds, in order.
pub fn detect_silence(
    samples: &[f32],
    sample_rate: u32,
    threshold_db: f64,
    min_duration: f64,
) -> Vec<(f64, f64)> {
    if samples.is_empty() || sample_rate == 0 {
        return Vec::new();
    }
    let threshold = 10f64.powf(threshold_db / 20.0) as f32;
    let window = ((sample_rate as f64 * 0.02) as usize).max(1); // 20ms
    let mut ranges = Vec::new();
    let mut run_start: Option<usize> = None;
    let mut i = 0;
    while i < samples.len() {
        let end = (i + window).min(samples.len());
        let sum_sq: f64 = samples[i..end].iter().map(|s| (*s as f64) * (*s as f64)).sum();
        let rms = (sum_sq / (end - i) as f64).sqrt() as f32;
        if rms < threshold {
            run_start.get_or_insert(i);
        } else if let Some(start) = run_start.take() {
            push_range(&mut ranges, start, i, sample_rate, min_duration);
        }
        i = end;
    }
    if let Some(start) = run_start {
        push_range(&mut ranges, start, samples.len(), sample_rate, min_duration);
    }
    ranges
}

fn push_range(ranges: &mut Vec<(f64, f64)>, start: usize, end: usize, sr: u32, min_duration: f64) {
    let s = start as f64 / sr as f64;
    let e = end as f64 / sr as f64;
    if e - s >= min_duration {
        ranges.push((s, e));
    }
}

/// Decode `uri`'s audio and detect silent ranges in it (media-relative
/// seconds). Synchronous and slow (decodes the whole file) — call from a
/// worker thread.
#[cfg(feature = "preview")]
pub fn detect_silence_in_uri(
    uri: &str,
    threshold_db: f64,
    min_duration: f64,
) -> anyhow::Result<Vec<(f64, f64)>> {
    let (samples, rate) = crate::thumbs::decode_mono_pcm(uri)?;
    Ok(detect_silence(&samples, rate, threshold_db, min_duration))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(secs: f64, sr: u32, amp: f32) -> Vec<f32> {
        vec![amp; (secs * sr as f64) as usize]
    }

    #[test]
    fn detects_a_silent_gap_between_two_loud_stretches() {
        let sr = 8000;
        let mut samples = tone(1.0, sr, 0.8);
        samples.extend(tone(1.0, sr, 0.0));
        samples.extend(tone(1.0, sr, 0.8));
        let ranges = detect_silence(&samples, sr, -40.0, 0.5);
        assert_eq!(ranges.len(), 1);
        let (s, e) = ranges[0];
        assert!((s - 1.0).abs() < 0.05, "start {s}");
        assert!((e - 2.0).abs() < 0.05, "end {e}");
    }

    #[test]
    fn ignores_silence_shorter_than_min_duration() {
        let sr = 8000;
        let mut samples = tone(1.0, sr, 0.8);
        samples.extend(tone(0.1, sr, 0.0));
        samples.extend(tone(1.0, sr, 0.8));
        assert!(detect_silence(&samples, sr, -40.0, 0.5).is_empty());
    }

    #[test]
    fn quiet_but_not_silent_audio_is_not_flagged() {
        let sr = 8000;
        // -20 dBFS is well above a -40 dBFS threshold.
        let samples = tone(1.0, sr, 0.1);
        assert!(detect_silence(&samples, sr, -40.0, 0.5).is_empty());
    }

    #[test]
    fn empty_input_returns_no_ranges() {
        assert!(detect_silence(&[], 8000, -40.0, 0.5).is_empty());
    }

    #[test]
    fn trailing_silence_at_end_of_stream_is_detected() {
        let sr = 8000;
        let mut samples = tone(1.0, sr, 0.8);
        samples.extend(tone(0.6, sr, 0.0));
        let ranges = detect_silence(&samples, sr, -40.0, 0.5);
        assert_eq!(ranges.len(), 1);
        assert!((ranges[0].0 - 1.0).abs() < 0.05);
    }
}
