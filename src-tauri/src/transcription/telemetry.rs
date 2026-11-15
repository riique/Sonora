//! History labels and latency metrics for the legacy pipeline.

/// Acoustic real-time factor: STT latency relative to audio length.
pub fn compute_realtime_factor(transcription_latency_ms: u64, duration_ms: u64) -> Option<f64> {
    if transcription_latency_ms > 0 && duration_ms > 0 {
        Some(transcription_latency_ms as f64 / duration_ms as f64)
    } else {
        None
    }
}

/// Estimated tokens/sec from word count and latency (legacy heuristic).
pub fn est_throughput(words: usize, latency_ms: u64) -> Option<f64> {
    if latency_ms > 0 {
        let est_tokens = words as f64 * 1.3;
        Some((est_tokens * 1000.0) / latency_ms as f64)
    } else {
        None
    }
}

pub fn est_total_tokens(words: usize) -> usize {
    (words as f64 * 1.3).round() as usize
}

pub fn log_latency(
    transcription_ms: u64,
    sanitizer_ms: u64,
    duration_ms: u64,
    realtime_factor: Option<f64>,
    deepgram_mode: Option<&str>,
    dual: bool,
    context: &str,
) {
    let total = transcription_ms + sanitizer_ms;
    if let Some(rtf) = realtime_factor {
        log::info!(
            "transcription: {} latency telemetry transcription_ms={} sanitizer_ms={} total_ms={} \
             duration_ms={} rtf={:.3} deepgram_mode={} dual={}",
            context,
            transcription_ms,
            sanitizer_ms,
            total,
            duration_ms,
            rtf,
            deepgram_mode.unwrap_or("-"),
            dual
        );
    } else {
        log::info!(
            "transcription: {} latency telemetry transcription_ms={} sanitizer_ms={} total_ms={} \
             deepgram_mode={} dual={}",
            context,
            transcription_ms,
            sanitizer_ms,
            total,
            deepgram_mode.unwrap_or("-"),
            dual
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtf_and_throughput() {
        assert_eq!(compute_realtime_factor(250, 1000), Some(0.25));
        assert_eq!(compute_realtime_factor(0, 1000), None);
        assert!(est_throughput(10, 1000).unwrap() > 0.0);
        assert_eq!(est_total_tokens(10), 13);
    }
}
