//! History entry helpers shared by the dictation and retry paths.

use crate::models::{AppState, HistoryEntry};
use crate::pipeline_contract::{GeminiProvider, TranscriptionMode};
use crate::pipeline_run::{
    epoch_ms, AttemptStatus, AudioTransport, PipelineError, PipelineErrorKind, PipelineRun,
    ProviderAttempt, StageKind, StageRecord, PIPELINE_RUN_SCHEMA_VERSION,
};

fn failed_run(state: &AppState, history_id: &str, error_msg: &str) -> PipelineRun {
    let now = epoch_ms();
    let error = PipelineError {
        kind: PipelineErrorKind::Provider,
        code: "recognition_failed".into(),
        message: error_msg.to_string(),
        retryable: true,
    };
    let mut run = PipelineRun::hard_error(
        format!("{history_id}-run-{now}"),
        TranscriptionMode::FastAccurate,
        error_msg.to_string(),
    );
    run.session_id = format!("{history_id}-session");
    run.started_at_ms = now;
    run.error = Some(error.clone());
    let choice = state.gemini_pipelines.read().fast_accurate.clone();
    let provider = match choice.provider {
        GeminiProvider::GoogleAiStudio => "google-ai-studio",
        GeminiProvider::OpenRouter => "openrouter",
    };
    run.add_attempt(ProviderAttempt {
        id: format!("{}-attempt-1", run.id),
        provider: provider.into(),
        model: choice.model_id().into(),
        transport: AudioTransport::InlineBase64,
        started_at_ms: now,
        duration_ms: Some(0),
        status: AttemptStatus::Failed,
        error: Some(error.clone()),
        ..ProviderAttempt::default()
    });
    run.add_stage(StageRecord::failed(StageKind::Recognition, 0, error));
    run
}

/// Rewrites an existing entry as failed (retry path).
pub fn update_failed_entry(
    state: &AppState,
    entry: &HistoryEntry,
    error_msg: String,
) -> HistoryEntry {
    let mut pipeline_runs = entry.pipeline_runs.clone();
    pipeline_runs.push(failed_run(state, &entry.id, &error_msg));
    HistoryEntry {
        schema_version: PIPELINE_RUN_SCHEMA_VERSION,
        id: entry.id.clone(),
        date: entry.date.clone(),
        words: 0,
        engine: entry.engine.clone(),
        text: String::new(),
        audio_path: entry.audio_path.clone(),
        evaluation: None,
        duration_ms: entry.duration_ms,
        source: entry.source.clone(),
        latency_ms: 0,
        throughput: 0.0,
        transcription_latency_ms: None,
        sanitizer_latency_ms: None,
        transcription_throughput: None,
        sanitizer_throughput: None,
        realtime_factor: None,
        deepgram_mode: None,
        total_tokens: None,
        is_error: Some(true),
        error_message: Some(error_msg),
        debug_info: None,
        mode: None,
        model: None,
        stages: None,
        used_fallback: None,
        fallback_reason: None,
        content_type: None,
        whisper_text: None,
        sanitizer_text: None,
        gemini_text: None,
        warnings: None,
        audio_prepare_ms: None,
        base64_ms: None,
        whisper_ms: None,
        sanitizer_ms: None,
        files_upload_ms: None,
        files_poll_ms: None,
        files_poll_count: None,
        gemini_generate_ms: None,
        gemini_delete_ms: None,
        strict_literals_ms: None,
        clipboard_ms: None,
        total_pipeline_ms: None,
        gemini_transport: None,
        pipeline_runs,
    }
}

/// Emits `transcription-saved` to the UI.
pub fn emit_saved(state: &AppState, entry: &HistoryEntry) {
    if let Some(handle) = state.app_handle.read().as_ref() {
        use tauri::Emitter;
        if let Err(e) = handle.emit(crate::models::event_names::TRANSCRIPTION_SAVED, entry) {
            log::warn!("transcription: failed to emit transcription-saved: {}", e);
        }
    }
}
