//! Groq sanitizer (UltraPrecise stage 2) and history entry helpers.

use std::sync::Arc;

use crate::models::{AppState, HistoryEntry, TranscriptionEngine};
use crate::pipeline_contract::TranscriptionMode;
use crate::pipeline_run::{
    epoch_ms, AttemptStatus, AudioTransport, PipelineError, PipelineErrorKind, PipelineRun,
    ProviderAttempt, StageKind, StageRecord, PIPELINE_RUN_SCHEMA_VERSION,
};
use crate::transcription::fallback::{coalesce_empty_final, pick_raw_acoustic};
use crate::transcription::telemetry::{acoustic_word_count, history_deepgram_mode};
use crate::transcription::types::SanitizeOutcome;

fn provider_identity(engine: TranscriptionEngine) -> (&'static str, &'static str, AudioTransport) {
    match engine {
        TranscriptionEngine::GroqWhisper => {
            ("groq", "whisper-large-v3-turbo", AudioTransport::Multipart)
        }
        TranscriptionEngine::DeepgramNova3 => ("deepgram", "nova-3", AudioTransport::RawBinary),
        TranscriptionEngine::GeminiMultimodal => (
            "google-ai-studio",
            "gemini-audio",
            AudioTransport::InlineBase64,
        ),
    }
}
fn legacy_failed_run(
    history_id: &str,
    engine: TranscriptionEngine,
    dual_mode: bool,
    error_msg: &str,
) -> PipelineRun {
    let now = epoch_ms();
    let mode = TranscriptionMode::from_legacy(engine, dual_mode);
    let error = PipelineError {
        kind: PipelineErrorKind::Provider,
        code: "recognition_failed".into(),
        message: error_msg.to_string(),
        retryable: true,
    };
    let mut run = PipelineRun::hard_error(
        format!("{history_id}-run-{now}"),
        mode,
        error_msg.to_string(),
    );
    run.session_id = format!("{history_id}-session");
    run.started_at_ms = now;
    run.error = Some(error.clone());
    let (provider, model, transport) = provider_identity(engine);
    run.add_attempt(ProviderAttempt {
        id: format!("{}-attempt-1", run.id),
        provider: provider.into(),
        model: model.into(),
        transport,
        started_at_ms: now,
        duration_ms: Some(0),
        status: AttemptStatus::Failed,
        error: Some(error.clone()),
        ..ProviderAttempt::default()
    });
    run.add_stage(StageRecord::failed(StageKind::Recognition, 0, error));
    run
}

/// Runs the Groq sanitizer (UltraPrecise stage 2), or keeps the raw Whisper text.
pub async fn run_sanitize(state: &Arc<AppState>, whisper_text: &str) -> SanitizeOutcome {
    // Single acoustic stream since the Deepgram dual engine was retired; the
    // shared helpers still accept a second (empty) stream.
    let deepgram_text = "";
    let sanitizer_key = state.next_groq_key();
    let (
        model_id,
        supports_reasoning,
        system_prompt,
        glossary_block,
        vocab_snapshot,
        reasoning_enabled,
        reasoning_effort,
    ) = {
        let sanitizer = *state.sanitizer.read();
        let vocab = state.vocabulary.read().clone();
        (
            sanitizer.api_model_id(),
            sanitizer.supports_reasoning(),
            state.system_prompt.read().clone(),
            crate::vocabulary::format_glossary_for_prompt(&vocab),
            vocab,
            *state.reasoning_enabled.read(),
            state.reasoning_effort.read().clone(),
        )
    };

    let system_prompt_to_use = system_prompt;

    let raw_words = acoustic_word_count(whisper_text, deepgram_text);
    let context_preferences = state.context_preferences.read().clone();
    let context_block = state
        .recording_session
        .lock()
        .as_ref()
        .filter(|session| session.profile.allow_context_to_cloud)
        .and_then(|session| {
            crate::context::package_untrusted_context(&session.context, &context_preferences)
        });
    let mut debug_info = None;
    let mut warnings: Vec<String> = Vec::new();
    let mut used_raw_fallback = false;
    let mut changed = false;
    let start_sanitizer = std::time::Instant::now();

    let final_text = if !*state.sanitizer_enabled.read() {
        let picked = pick_raw_acoustic(whisper_text, deepgram_text);
        log::info!(
            "transcription: sanitizer disabled, using pick_raw ({} chars)",
            picked.len()
        );
        picked
    } else {
        match sanitizer_key {
            Some(key) => {
                let outcome = crate::groq::call_sanitizer_api(crate::groq::CallSanitizerApiInput {
                    whisper_text,
                    model: model_id,
                    system_prompt: &system_prompt_to_use,
                    untrusted_context: context_block.as_deref(),
                    glossary_block: &glossary_block,
                    api_key: &key,
                    reasoning_enabled,
                    reasoning_effort: &reasoning_effort,
                    reasoning_supported: supports_reasoning,
                })
                .await;

                debug_info = Some(outcome.debug);
                warnings.extend(outcome.warnings);
                changed = outcome.changed;
                used_raw_fallback = outcome.used_raw_fallback;
                match outcome.result {
                    Ok(sanitized)
                        if sanitized.trim() == crate::groq::FALLBACK_RETRY_SENTINEL
                            || outcome.used_raw_fallback
                            || sanitized.trim().is_empty() =>
                    {
                        log::warn!(
                            "transcription: sanitizer fallback to raw (sentinel/parse/empty)"
                        );
                        used_raw_fallback = true;
                        warnings.push("sanitizer_used_raw_fallback".into());
                        pick_raw_acoustic(whisper_text, deepgram_text)
                    }
                    Ok(sanitized) => {
                        log::info!(
                            "transcription: sanitizer structured text ({} chars, changed={})",
                            sanitized.len(),
                            changed
                        );
                        sanitized
                    }
                    Err(e) => {
                        log::error!("transcription: sanitizer failed: {e}; using pick_raw");
                        used_raw_fallback = true;
                        warnings.push(format!("sanitizer_error: {}", e));
                        pick_raw_acoustic(whisper_text, deepgram_text)
                    }
                }
            }
            None => {
                log::warn!(
                    "transcription: no sanitizer API key set for model, falling back to pick_raw"
                );
                used_raw_fallback = true;
                warnings.push("sanitizer_missing_api_key".into());
                pick_raw_acoustic(whisper_text, deepgram_text)
            }
        }
    };

    let final_text = coalesce_empty_final(final_text, whisper_text, deepgram_text);
    // Strict literals: deterministic alias→canonical only for unequivocal hits.
    let (final_text, strict_hits) =
        crate::vocabulary::apply_strict_literals(&final_text, &vocab_snapshot);
    if !strict_hits.is_empty() {
        log::info!(
            "transcription: strict literals applied ({})",
            strict_hits.join(", ")
        );
        warnings.push(format!("strict_literals:{}", strict_hits.len()));
    }
    let final_text = crate::transcription::remove_known_transcription_artifacts(&final_text);
    let sanitizer_latency_ms = start_sanitizer.elapsed().as_millis() as u64;

    SanitizeOutcome {
        final_text,
        debug_info,
        sanitizer_latency_ms,
        raw_words,
        warnings,
        used_raw_fallback,
        changed,
        // Kept only for compatibility with historical/debug structures. The
        // active prompt no longer branches on a user-selected content type.
        content_type: crate::pipeline_contract::ContentType::Auto,
    }
}

/// Rewrites an existing entry as failed (retry path).
pub fn update_failed_entry(
    state: &AppState,
    entry: &HistoryEntry,
    error_msg: String,
) -> HistoryEntry {
    let dual = *state.dual_engine.read();
    let eng = state.active_engine();
    let deepgram_attempted = dual || eng == TranscriptionEngine::DeepgramNova3;
    let mut pipeline_runs = entry.pipeline_runs.clone();
    pipeline_runs.push(legacy_failed_run(&entry.id, eng, dual, &error_msg));
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
        deepgram_mode: history_deepgram_mode(state, deepgram_attempted),
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
