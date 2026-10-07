//! Transcription orchestration: product modes, the shared Groq sanitizer,
//! history entry helpers and telemetry. Capture/WAV/clipboard live in `audio.rs`.

pub mod fallback;
pub mod legacy;
pub mod modes;
pub mod pipeline;
pub mod telemetry;
pub mod types;

pub use crate::pipeline_run::PipelineRun;
pub use fallback::{coalesce_empty_final, pick_raw_acoustic};
pub use legacy::{transcribe_groq_whisper, transcribe_whisper_fallback_via_openrouter};
pub use modes::{
    mode_failed_history, mode_result_to_history, run_product_mode, run_product_mode_with_duration,
};
pub use pipeline::{emit_saved, run_sanitize, update_failed_entry};
pub use types::SanitizeOutcome;

const KNOWN_TRANSCRIPTION_ARTIFACTS: &[&str] = &["Legenda por Sônia Ruberti"];

/// Removes deterministic phrases hallucinated by transcription engines before
/// the text is persisted, emitted to the UI, copied or pasted.
pub fn remove_known_transcription_artifacts(text: &str) -> String {
    let mut cleaned = text.to_string();
    for artifact in KNOWN_TRANSCRIPTION_ARTIFACTS {
        cleaned = cleaned.replace(artifact, "");
    }

    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }

    cleaned.trim().to_string()
}

#[cfg(test)]
mod artifact_tests {
    use super::remove_known_transcription_artifacts;

    #[test]
    fn removes_sonia_ruberti_caption_artifact() {
        assert_eq!(
            remove_known_transcription_artifacts("Legenda por Sônia Ruberti"),
            ""
        );
        assert_eq!(
            remove_known_transcription_artifacts(
                "Texto antes. Legenda por Sônia Ruberti Texto depois."
            ),
            "Texto antes. Texto depois."
        );
    }

    #[test]
    fn preserves_unrelated_transcription() {
        let text = "Esta é uma transcrição normal.";
        assert_eq!(remove_known_transcription_artifacts(text), text);
    }
}
