//! Product modes: UltraFast, FastAccurate, Precise, UltraPrecise.

use std::sync::Arc;

use crate::gemini::{
    adaptive_generate_timeout, fast_accurate_transcription_prompt, transcribe_audio,
    GeminiAudioTransport, GeminiGenerateResult, GeminiOperation, GeminiPrompt, GeminiStageTiming,
    TranscribeRequest, TRANSCRIBE_PROMPT_VERSION,
};
use crate::models::{AppState, HistoryEntry, SanitizerDebug};
use crate::pipeline_contract::{GeminiProvider, TranscriptionMode};
use crate::pipeline_run::{
    AttemptResultMetadata, AttemptStatus, AudioTransport, CostKind, CostRecord, PipelineError,
    PipelineErrorKind, PipelineRun, ProviderAttempt, StageKind, StageRecord, UsageRecord,
};
use crate::transcription::telemetry::{
    compute_realtime_factor, est_throughput, est_total_tokens, log_latency,
};

fn provider_history_label(provider: GeminiProvider) -> &'static str {
    match provider {
        GeminiProvider::GoogleAiStudio => "GoogleAIStudio",
        GeminiProvider::OpenRouter => "OpenRouter",
    }
}

fn emit_fallback_progress(state: &AppState, provider: &str, fallback_provider: &str, reason: &str) {
    log::warn!("modes: {provider} failed, falling back to {fallback_provider}: {reason}");
    crate::pipeline_run::emit_pipeline_progress(
        state,
        crate::pipeline_run::PipelineProgressEvent {
            kind: crate::pipeline_run::PipelineProgressKind::ProviderFailed,
            provider: Some(provider.to_string()),
            fallback_provider: Some(fallback_provider.to_string()),
            message: Some(reason.to_string()),
            ..Default::default()
        },
    );
    crate::pipeline_run::emit_pipeline_progress(
        state,
        crate::pipeline_run::PipelineProgressEvent {
            kind: crate::pipeline_run::PipelineProgressKind::FallbackStarted,
            provider: Some(provider.to_string()),
            fallback_provider: Some(fallback_provider.to_string()),
            message: Some(format!(
                "{provider} indisponível · usando {fallback_provider}"
            )),
            ..Default::default()
        },
    );
}

fn audio_transport_from_gemini(
    value: Option<GeminiAudioTransport>,
    usage: &UsageRecord,
) -> AudioTransport {
    if let Some(transport) = usage
        .metadata
        .get("transport")
        .and_then(serde_json::Value::as_str)
    {
        return match transport {
            "multipart" => AudioTransport::Multipart,
            "raw_binary" => AudioTransport::RawBinary,
            "resumable_file" => AudioTransport::ResumableFile,
            "url" => AudioTransport::Url,
            "websocket_stream" => AudioTransport::WebSocketStream,
            _ => AudioTransport::InlineBase64,
        };
    }
    match value {
        Some(GeminiAudioTransport::FilesApi) => AudioTransport::ResumableFile,
        Some(GeminiAudioTransport::Inline) | None => AudioTransport::InlineBase64,
    }
}

fn completed_attempt(
    id: &str,
    provider: &str,
    model: &str,
    transport: AudioTransport,
    duration_ms: u64,
    output_chars: usize,
    bytes_sent: usize,
) -> ProviderAttempt {
    ProviderAttempt {
        id: id.into(),
        provider: provider.into(),
        model: model.into(),
        transport,
        started_at_ms: crate::pipeline_run::epoch_ms().saturating_sub(duration_ms),
        duration_ms: Some(duration_ms),
        status: AttemptStatus::Success,
        usage: UsageRecord {
            bytes_sent: (bytes_sent > 0).then_some(bytes_sent as u64),
            ..Default::default()
        },
        result: AttemptResultMetadata {
            output_chars: Some(output_chars),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn failed_attempt(
    id: &str,
    provider: &str,
    model: &str,
    transport: AudioTransport,
    duration_ms: u64,
    code: &str,
    message: &str,
) -> ProviderAttempt {
    let kind = if code == "missing_key" {
        PipelineErrorKind::Authentication
    } else {
        PipelineErrorKind::Provider
    };
    ProviderAttempt {
        id: id.into(),
        provider: provider.into(),
        model: model.into(),
        transport,
        started_at_ms: crate::pipeline_run::epoch_ms().saturating_sub(duration_ms),
        duration_ms: Some(duration_ms),
        status: AttemptStatus::Failed,
        error: Some(PipelineError {
            kind,
            code: code.into(),
            message: message.into(),
            retryable: kind != PipelineErrorKind::Authentication,
        }),
        ..Default::default()
    }
}
pub struct OpenrouterAudioResultInput<'a> {
    pub state: &'a AppState,
    pub audio: &'a [u8],
    pub ext: &'a str,
    pub prompt: &'a GeminiPrompt,
    pub model: &'a str,
    pub api_key: &'a str,
    pub duration_ms: Option<u64>,
    pub operation: GeminiOperation,
    pub prompt_version: &'a str,
}

async fn openrouter_audio_result(
    input: OpenrouterAudioResultInput<'_>,
) -> Result<GeminiGenerateResult, String> {
    let OpenrouterAudioResultInput {
        state,
        audio,
        ext,
        prompt,
        model,
        api_key,
        duration_ms,
        operation,
        prompt_version,
    } = input;
    let adaptive_timeout = adaptive_generate_timeout(duration_ms, audio.len());
    let route = crate::openrouter::detect_audio_route(model).await;
    log::info!(
        "modes: OpenRouter automatic audio route model={} endpoint={}",
        model,
        route.as_str()
    );
    let out = match route {
        crate::openrouter::OpenRouterAudioRoute::MultimodalLlm => {
            crate::openrouter::generate_with_audio(
                audio,
                ext,
                &prompt.system_instruction,
                &prompt.user_prompt,
                model,
                api_key,
                adaptive_timeout,
            )
            .await?
        }
        crate::openrouter::OpenRouterAudioRoute::SpeechToText => {
            let assembly_options = if crate::smart_context::is_assemblyai(model) {
                let prepared = state
                    .smart_context_output
                    .read()
                    .clone()
                    .unwrap_or_default();
                let smart_config = state.smart_context_config.read().clone();
                // Preserve recent speech context even when visual analysis takes too long.
                let conversation_context =
                    if smart_config.enabled && prepared.conversation_context.is_empty() {
                        crate::smart_context::previous_dictations(smart_config.history_count)
                    } else {
                        prepared.conversation_context
                    };
                let vocab = state.vocabulary.read();
                Some(crate::openrouter::AssemblyAiOptions {
                    prompt: crate::smart_context::prompt(prepared.prompt.as_deref()),
                    keyterms_prompt: crate::smart_context::keyterms(&vocab, &prepared.keyterms),
                    conversation_context,
                })
            } else {
                None
            };
            crate::openrouter::transcribe_audio_with_assembly(
                audio,
                ext,
                model,
                api_key,
                std::time::Duration::from_secs(120).max(adaptive_timeout),
                None,
                assembly_options.as_ref(),
            )
            .await?
        }
    };
    let mut usage = UsageRecord {
        audio_seconds: out.reported_audio_seconds,
        input_tokens: out.reported_input_tokens.map(|value| value as u64),
        output_tokens: out.reported_output_tokens.map(|value| value as u64),
        total_tokens: out.reported_total_tokens.map(|value| value as u64),
        bytes_sent: Some(out.bytes_sent),
        cost: out
            .reported_cost_usd
            .map_or_else(CostRecord::default, |amount| CostRecord {
                kind: CostKind::Actual,
                amount_usd: Some(amount),
                source: Some("openrouter_response_usage".into()),
            }),
        ..Default::default()
    };
    usage
        .metadata
        .insert("transport".into(), out.transport.as_str().into());
    usage
        .metadata
        .insert("request_ms".into(), out.request_ms.into());
    if let Some(ttfb_ms) = out.ttfb_ms {
        usage.metadata.insert("ttfb_ms".into(), ttfb_ms.into());
    }
    Ok(GeminiGenerateResult {
        operation,
        text: out.text,
        model: model.to_string(),
        prompt_version: prompt_version.to_string(),
        latency_ms: out.base64_ms + out.request_ms,
        remote_file_name: None,
        transport: Some(GeminiAudioTransport::Inline),
        timing: GeminiStageTiming {
            base64_ms: Some(out.base64_ms),
            generate_ms: Some(out.request_ms),
            ..Default::default()
        },
        usage,
    })
}

struct WhisperFallbackResult {
    model: &'static str,
    text: String,
    generated: crate::openrouter::OpenRouterGenerateResult,
}

async fn run_openrouter_whisper_fallback(
    state: &Arc<AppState>,
    audio: &[u8],
    ext: &str,
) -> Result<WhisperFallbackResult, String> {
    let (model, generated) =
        crate::transcription::transcribe_whisper_fallback_via_openrouter(state, audio, ext).await?;
    let text = generated.text.trim().to_string();
    if text.is_empty() {
        return Err("OpenRouter STT não retornou texto.".to_string());
    }
    Ok(WhisperFallbackResult {
        model,
        text,
        generated,
    })
}

fn openrouter_usage(out: &crate::openrouter::OpenRouterGenerateResult) -> UsageRecord {
    UsageRecord {
        audio_seconds: out.reported_audio_seconds,
        input_tokens: out.reported_input_tokens.map(|value| value as u64),
        output_tokens: out.reported_output_tokens.map(|value| value as u64),
        total_tokens: out.reported_total_tokens.map(|value| value as u64),
        bytes_sent: Some(out.bytes_sent),
        cost: out
            .reported_cost_usd
            .map_or_else(CostRecord::default, |amount| CostRecord {
                kind: CostKind::Actual,
                amount_usd: Some(amount),
                source: Some("openrouter_response_usage".into()),
            }),
        metadata: {
            let mut metadata = std::collections::BTreeMap::new();
            metadata.insert("request_ms".into(), out.request_ms.into());
            if let Some(ttfb_ms) = out.ttfb_ms {
                metadata.insert("ttfb_ms".into(), ttfb_ms.into());
            }
            metadata.insert("transport".into(), out.transport.as_str().into());
            metadata
        },
    }
}

fn pipeline_debug_snapshot(result: &PipelineRun) -> SanitizerDebug {
    let mut user_parts = Vec::new();
    if let Some(w) = result.whisper_text.as_ref().filter(|s| !s.is_empty()) {
        user_parts.push(format!("[WHISPER]\n{}", w));
    }
    if let Some(s) = result.sanitizer_text.as_ref().filter(|t| !t.is_empty()) {
        user_parts.push(format!("[SANITIZER]\n{}", s));
    }
    if let Some(g) = result.gemini_text.as_ref().filter(|t| !t.is_empty()) {
        user_parts.push(format!("[GEMINI]\n{}", g));
    }
    if user_parts.is_empty() {
        user_parts.push(format!("[FINAL]\n{}", result.final_text));
    }
    let request_json = serde_json::json!({
        "kind": "product_pipeline",
        "mode": result.mode.as_str(),
        "model": result.model,
        "stages": result.stages,
        "transport": result.gemini_transport,
        "used_fallback": result.used_fallback,
        "fallback_reason": result.fallback_reason,
        "whisper_ms": result.whisper_ms,
        "sanitizer_ms": result.sanitizer_ms,
        "base64_ms": result.base64_ms,
        "files_upload_ms": result.files_upload_ms,
        "files_poll_ms": result.files_poll_ms,
        "files_poll_count": result.files_poll_count,
        "gemini_generate_ms": result.gemini_generate_ms,
        "total_pipeline_ms": result.total_pipeline_ms,
        "openrouter_generation_id": result.openrouter_generation_id,
        "reported_total_tokens": result.reported_total_tokens,
    });
    SanitizerDebug {
        endpoint: format!("product-mode:{}", result.mode.as_str()),
        model: result.model.clone(),
        temperature: 0.0,
        reasoning_enabled: false,
        reasoning_effort: String::new(),
        reasoning_effort_applied: false,
        reasoning_supported_by_model: false,
        system_prompt: format!(
            "Pipeline de produto.\nTransporte: {:?}\nEtapas: {}",
            result.gemini_transport,
            result.stages.join(" → ")
        ),
        user_message: user_parts.join("\n\n"),
        request_json: serde_json::to_string_pretty(&request_json).unwrap_or_else(|_| "{}".into()),
        response_status: Some(200),
        response_content: Some(result.final_text.clone()),
        response_reasoning: None,
        error: result.fallback_reason.clone(),
    }
}

fn apply_timing_from_gemini(result: &mut PipelineRun, g: &crate::gemini::GeminiGenerateResult) {
    result.usage.merge(&g.usage);
    if let Some(transport) = g
        .usage
        .metadata
        .get("transport")
        .and_then(serde_json::Value::as_str)
    {
        result.gemini_transport = Some(transport.to_string());
    } else if let Some(t) = g.transport {
        result.gemini_transport = Some(t.as_str().to_string());
    }
    result.base64_ms = g.timing.base64_ms.or(result.base64_ms);
    result.files_upload_ms = g.timing.files_upload_ms.or(result.files_upload_ms);
    result.files_poll_ms = g.timing.files_poll_ms.or(result.files_poll_ms);
    result.files_poll_count = g.timing.files_poll_count.or(result.files_poll_count);
    result.gemini_generate_ms = g.timing.generate_ms.or(result.gemini_generate_ms);
    result.gemini_delete_ms = g.timing.delete_ms.or(result.gemini_delete_ms);
    result.gemini_ms = g.timing.generate_ms.or(Some(g.latency_ms));
    result.timings.request_ms = g
        .usage
        .metadata
        .get("request_ms")
        .and_then(serde_json::Value::as_u64);
    result.timings.ttfb_ms = g
        .usage
        .metadata
        .get("ttfb_ms")
        .and_then(serde_json::Value::as_u64);
    if let Some(u) = g.timing.files_upload_ms {
        let poll = g.timing.files_poll_ms.unwrap_or(0);
        result.upload_ms = Some(u + poll);
    }
}

/// Usage record for a text-only assist call (quick refine, voice command).
pub(crate) fn assist_usage(out: &crate::assist::LlmText) -> UsageRecord {
    UsageRecord {
        input_tokens: out.input_tokens,
        output_tokens: out.output_tokens,
        cost: out
            .cost_usd
            .map_or_else(CostRecord::default, |amount| CostRecord {
                kind: CostKind::Actual,
                amount_usd: Some(amount),
                source: Some("openrouter_response_usage".into()),
            }),
        ..Default::default()
    }
}

/// Records a successful assist call as a provider attempt plus a refinement stage,
/// so the history shows which model touched the text and what it cost.
pub(crate) fn record_assist_call(run: &mut PipelineRun, id: &str, out: &crate::assist::LlmText) {
    let usage = assist_usage(out);
    let mut attempt = completed_attempt(
        id,
        out.provider,
        out.model,
        AudioTransport::InlineBase64,
        out.duration_ms,
        out.text.len(),
        0,
    );
    attempt.usage = usage.clone();
    run.attempts.push(attempt);
    let mut stage = StageRecord::completed(StageKind::SemanticRefinement, out.duration_ms);
    stage.id = format!("stage-{id}");
    stage.provider = Some(out.provider.into());
    stage.model = Some(out.model.into());
    stage.usage = usage;
    run.journal.push(stage);
}

/// FastAccurate: Gemini (inline or Files) + glossary + strict literals.
/// Runs the Whisper backup after the audio model failed, recording the attempt.
/// Returns `(text, model)` or the Whisper error.
async fn whisper_backup(
    state: &Arc<AppState>,
    audio: &[u8],
    ext: &str,
    reason: &str,
    attempts: &mut Vec<ProviderAttempt>,
    result_meta: &mut PipelineRun,
    stages: &mut Vec<String>,
) -> Result<(String, String), String> {
    emit_fallback_progress(state, "Gemini", "OpenRouter/Groq Whisper", reason);
    let started = std::time::Instant::now();
    match run_openrouter_whisper_fallback(state, audio, ext).await {
        Ok(outcome) => {
            let whisper_ms = started.elapsed().as_millis() as u64;
            let mut attempt = completed_attempt(
                "attempt-2",
                "openrouter/groq",
                outcome.model,
                outcome.generated.transport,
                whisper_ms,
                outcome.text.len(),
                outcome.generated.bytes_sent as usize,
            );
            attempt.usage = openrouter_usage(&outcome.generated);
            attempts.push(attempt);
            result_meta.whisper_ms = Some(whisper_ms);
            stages.push("whisper_fallback:openrouter/groq".into());
            Ok((outcome.text, outcome.model.to_string()))
        }
        Err(error) => {
            attempts.push(failed_attempt(
                "attempt-2",
                "openrouter/groq",
                crate::transcription::legacy::WHISPER_FALLBACK_MODEL,
                AudioTransport::Multipart,
                started.elapsed().as_millis() as u64,
                "provider_error",
                &error,
            ));
            Err(error)
        }
    }
}

pub async fn run_fast_accurate(
    state: &Arc<AppState>,
    audio: Vec<u8>,
    ext: &str,
    file_name: &str,
    duration_ms: Option<u64>,
) -> Result<PipelineRun, String> {
    let t0 = std::time::Instant::now();
    let audio_len = audio.len();
    let choice = state.gemini_pipelines.read().fast_accurate.clone();
    let model_id = choice.model_id().to_string();
    let api_key = match choice.provider {
        GeminiProvider::GoogleAiStudio => state.next_google_key(),
        GeminiProvider::OpenRouter => state.next_openrouter_key(),
    };
    let glossary_block = {
        let vocab = state.vocabulary.read().clone();
        crate::vocabulary::format_glossary_for_prompt(&vocab)
    };
    let provider_label = provider_history_label(choice.provider);
    log::info!(
        "modes: FastAccurate → {} model={}",
        provider_label,
        model_id
    );

    let mut stages = Vec::new();
    let mut whisper_text = None;
    let mut gemini_text = None;
    let mut result_meta = PipelineRun::default();
    let mut attempts = Vec::new();
    let gemini_provider = match choice.provider {
        GeminiProvider::GoogleAiStudio => "google-ai-studio",
        GeminiProvider::OpenRouter => "openrouter",
    };

    // Ok((text, model)) from the audio model, or Err((fallback_reason, message)).
    let primary: Result<(String, String), (String, String)> = match api_key {
        None => {
            stages.push("gemini_skipped_no_key".into());
            attempts.push(failed_attempt(
                "attempt-1",
                gemini_provider,
                &model_id,
                AudioTransport::InlineBase64,
                0,
                "missing_key",
                "Chave do provedor não configurada",
            ));
            Err((
                "gemini_missing_api_key".into(),
                "Chave do provedor selecionado ausente".into(),
            ))
        }
        Some(key) => {
            stages.push(format!(
                "model_api:{}",
                if choice.provider == GeminiProvider::OpenRouter {
                    "automatic"
                } else if crate::gemini::is_transcribe_model(&model_id) {
                    "google-interactions"
                } else {
                    "google-generate-content"
                }
            ));
            stages.push("gemini_transcribe".into());
            let custom_vocabulary: Vec<String> = {
                let vocab = state.vocabulary.read();
                vocab
                    .iter()
                    .filter(|t| t.enabled)
                    .flat_map(|t| std::iter::once(t.canonical.clone()).chain(t.aliases.clone()))
                    .take(1000)
                    .collect()
            };
            let provider_started = std::time::Instant::now();
            let generated = if choice.provider == GeminiProvider::OpenRouter {
                let prompt = fast_accurate_transcription_prompt(&glossary_block);
                openrouter_audio_result(OpenrouterAudioResultInput {
                    state,
                    audio: &audio,
                    ext,
                    prompt: &prompt,
                    model: &model_id,
                    api_key: &key,
                    duration_ms,
                    operation: GeminiOperation::Transcribe,
                    prompt_version: TRANSCRIBE_PROMPT_VERSION,
                })
                .await
            } else {
                transcribe_audio(TranscribeRequest {
                    audio_bytes: audio.clone(),
                    ext: ext.to_string(),
                    api_key: key.clone(),
                    model: model_id.clone(),
                    display_name: file_name.to_string(),
                    duration_ms,
                    glossary_block,
                    custom_vocabulary,
                })
                .await
            };
            let provider_ms = provider_started.elapsed().as_millis() as u64;
            match generated {
                Ok(r) if !r.text.trim().is_empty() => {
                    apply_timing_from_gemini(&mut result_meta, &r);
                    let mut attempt = completed_attempt(
                        "attempt-1",
                        gemini_provider,
                        &r.model,
                        audio_transport_from_gemini(r.transport, &r.usage),
                        provider_ms,
                        r.text.len(),
                        audio_len,
                    );
                    attempt.usage = r.usage.clone();
                    attempts.push(attempt);
                    if let Some(t) = r.transport {
                        stages.push(format!("transport:{}", t.as_str()));
                    }
                    gemini_text = Some(r.text.clone());
                    Ok((r.text, r.model))
                }
                Ok(r) => {
                    attempts.push(failed_attempt(
                        "attempt-1",
                        gemini_provider,
                        &r.model,
                        audio_transport_from_gemini(r.transport, &r.usage),
                        provider_ms,
                        "empty_response",
                        "O provider retornou texto vazio",
                    ));
                    Err((
                        "gemini_empty".into(),
                        "Resposta vazia do provedor selecionado".into(),
                    ))
                }
                Err(e) => {
                    attempts.push(failed_attempt(
                        "attempt-1",
                        gemini_provider,
                        &model_id,
                        AudioTransport::InlineBase64,
                        provider_ms,
                        "provider_error",
                        &e,
                    ));
                    Err((format!("gemini_error: {e}"), e))
                }
            }
        }
    };

    let mut used_fallback = false;
    let mut fallback_reason = None;
    let mut label = format!("FastAccurate/{provider_label}");
    let (final_text, model) = match primary {
        Ok(done) => done,
        Err((reason, message)) => {
            used_fallback = true;
            fallback_reason = Some(reason);
            label = "FastAccurate/WhisperFallback/OpenRouter-Groq".into();
            match whisper_backup(
                state,
                &audio,
                ext,
                &message,
                &mut attempts,
                &mut result_meta,
                &mut stages,
            )
            .await
            {
                Ok((text, model)) => {
                    whisper_text = Some(text.clone());
                    (text, model)
                }
                Err(whisper_error) => {
                    let message = format!(
                        "O modelo falhou ({message}) e o Whisper de reserva também: {whisper_error}"
                    );
                    remember_failed_attempts(
                        state,
                        TranscriptionMode::FastAccurate,
                        attempts,
                        &message,
                        true,
                        Some("both_providers_failed".into()),
                    );
                    return Err(message);
                }
            }
        }
    };

    if final_text.trim().is_empty() {
        return Err("Nenhum texto detectado na gravação.".to_string());
    }

    let ms = t0.elapsed().as_millis() as u64;
    Ok(PipelineRun {
        final_text: final_text.trim().to_string(),
        mode: TranscriptionMode::FastAccurate,
        model,
        stages,
        used_fallback,
        fallback_reason,
        whisper_text,
        gemini_text,
        transcription_latency_ms: ms,
        history_engine_label: label,
        total_pipeline_ms: Some(ms),
        whisper_ms: result_meta.whisper_ms,
        upload_ms: result_meta.upload_ms,
        gemini_ms: result_meta.gemini_ms,
        base64_ms: result_meta.base64_ms,
        files_upload_ms: result_meta.files_upload_ms,
        files_poll_ms: result_meta.files_poll_ms,
        files_poll_count: result_meta.files_poll_count,
        gemini_generate_ms: result_meta.gemini_generate_ms,
        gemini_transport: result_meta.gemini_transport,
        attempts,
        ..Default::default()
    })
}

/// Builds a history entry from a mode result.
pub fn mode_result_to_history(
    id: String,
    date: String,
    audio_path: Option<String>,
    duration_ms: u64,
    source: &str,
    result: &PipelineRun,
) -> HistoryEntry {
    let mut canonical_run = result.clone();
    if canonical_run.id.is_empty() {
        canonical_run.id = format!("{id}-run-{}", crate::pipeline_run::epoch_ms());
    }
    if canonical_run.session_id.is_empty() {
        canonical_run.session_id = format!("{id}-session");
    }
    if canonical_run.started_at_ms == 0 {
        canonical_run.started_at_ms =
            crate::pipeline_run::epoch_ms().saturating_sub(canonical_run.transcription_latency_ms);
    }
    canonical_run.normalize();
    if canonical_run.finished_at_ms.is_none() {
        canonical_run.finish_success();
    }
    let result = &canonical_run;
    let words = result.final_text.split_whitespace().count();
    let transcription_throughput = est_throughput(words, result.transcription_latency_ms);
    let realtime_factor = compute_realtime_factor(result.transcription_latency_ms, duration_ms);

    let san_ms = result.sanitizer_ms;
    log_latency(
        result
            .transcription_latency_ms
            .saturating_sub(san_ms.unwrap_or(0)),
        san_ms.unwrap_or(0),
        duration_ms,
        realtime_factor,
        None,
        false,
        result.mode.as_str(),
    );
    log::info!(
        "modes: {} breakdown whisper={:?} sanitizer={:?} base64={:?} upload={:?} poll={:?} generate={:?} transport={:?} total={}",
        result.mode.as_str(),
        result.whisper_ms,
        result.sanitizer_ms,
        result.base64_ms,
        result.files_upload_ms,
        result.files_poll_ms,
        result.gemini_generate_ms,
        result.gemini_transport,
        result.transcription_latency_ms
    );

    let mut warnings = result.warnings.clone();
    if result.used_fallback {
        if let Some(r) = &result.fallback_reason {
            if !warnings.iter().any(|w| w == r) {
                warnings.push(r.clone());
            }
        }
    }

    HistoryEntry {
        schema_version: crate::pipeline_run::PIPELINE_RUN_SCHEMA_VERSION,
        id,
        date,
        words,
        engine: result.history_engine_label.clone(),
        text: result.final_text.clone(),
        audio_path,
        evaluation: None,
        duration_ms,
        source: source.to_string(),
        latency_ms: result.transcription_latency_ms,
        throughput: transcription_throughput.unwrap_or(0.0),
        transcription_latency_ms: Some(result.transcription_latency_ms),
        // Real sanitizer time when measured; None when mode has no sanitizer.
        sanitizer_latency_ms: san_ms,
        transcription_throughput,
        sanitizer_throughput: san_ms.and_then(|ms| {
            if ms == 0 {
                None
            } else {
                est_throughput(words, ms)
            }
        }),
        realtime_factor,
        deepgram_mode: None,
        total_tokens: result
            .reported_total_tokens
            .or_else(|| Some(est_total_tokens(words))),
        is_error: Some(result.delivery.error.is_some()),
        error_message: result
            .delivery
            .error
            .as_ref()
            .map(|error| error.message.clone()),
        debug_info: result
            .debug_info
            .clone()
            .or_else(|| Some(pipeline_debug_snapshot(result))),
        mode: Some(result.mode.as_str().to_string()),
        model: Some(result.model.clone()),
        stages: Some(result.stages.join(",")),
        used_fallback: Some(result.used_fallback),
        fallback_reason: result.fallback_reason.clone(),
        content_type: result.content_type.clone(),
        whisper_text: result.whisper_text.clone(),
        sanitizer_text: result.sanitizer_text.clone(),
        gemini_text: result.gemini_text.clone(),
        warnings: if warnings.is_empty() {
            None
        } else {
            Some(warnings)
        },
        audio_prepare_ms: result.audio_prepare_ms,
        base64_ms: result.base64_ms,
        whisper_ms: result.whisper_ms,
        sanitizer_ms: result.sanitizer_ms,
        files_upload_ms: result.files_upload_ms,
        files_poll_ms: result.files_poll_ms,
        files_poll_count: result.files_poll_count,
        gemini_generate_ms: result.gemini_generate_ms,
        gemini_delete_ms: result.gemini_delete_ms,
        strict_literals_ms: result.strict_literals_ms,
        clipboard_ms: None,
        total_pipeline_ms: result
            .total_pipeline_ms
            .or(Some(result.transcription_latency_ms)),
        gemini_transport: result.gemini_transport.clone(),
        pipeline_runs: vec![canonical_run],
    }
}

pub fn mode_failed_history(
    id: String,
    date: String,
    audio_path: Option<String>,
    duration_ms: u64,
    source: &str,
    mode: TranscriptionMode,
    error_msg: String,
) -> HistoryEntry {
    let mut run = PipelineRun::hard_error(
        format!("{id}-run-{}", crate::pipeline_run::epoch_ms()),
        mode,
        error_msg.clone(),
    );
    run.session_id = format!("{id}-session");
    HistoryEntry {
        schema_version: crate::pipeline_run::PIPELINE_RUN_SCHEMA_VERSION,
        id,
        date,
        words: 0,
        engine: format!("{}/error", mode.as_str()),
        text: String::new(),
        audio_path,
        evaluation: None,
        duration_ms,
        source: source.to_string(),
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
        mode: Some(mode.as_str().to_string()),
        model: None,
        stages: None,
        used_fallback: Some(false),
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
        pipeline_runs: vec![run],
    }
}

pub async fn run_product_mode_with_duration(
    state: &Arc<AppState>,
    audio: Vec<u8>,
    file_name: &str,
    mime: &str,
    ext: &str,
    duration_ms: Option<u64>,
) -> Result<PipelineRun, String> {
    let original_state = state;
    let snapshot = state.pipeline_snapshot();
    let state = &snapshot;
    state.pending_failed_pipeline_run.lock().take();
    let mode = TranscriptionMode::FastAccurate;
    let _ = mime;
    let result = run_fast_accurate(state, audio, ext, file_name, duration_ms).await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            if state.pending_failed_pipeline_run.lock().is_none() {
                remember_failed_attempts(state, mode, Vec::new(), &error, false, None);
            }
            *original_state.pending_failed_pipeline_run.lock() =
                state.pending_failed_pipeline_run.lock().take();
            return Err(error);
        }
    };
    Ok(finalize_product_result(state, result))
}

fn remember_failed_attempts(
    state: &AppState,
    mode: TranscriptionMode,
    mut attempts: Vec<ProviderAttempt>,
    message: &str,
    fallback_used: bool,
    fallback_reason: Option<String>,
) {
    if attempts.is_empty() {
        let (provider, model, transport) =
            ("gemini", "configured-model", AudioTransport::InlineBase64);
        attempts.push(failed_attempt(
            "attempt-1",
            provider,
            model,
            transport,
            0,
            "pipeline_error",
            message,
        ));
    }
    let mut run = PipelineRun::hard_error("", mode, message);
    run.attempts = attempts;
    run.fallback.used = fallback_used;
    run.fallback.reason = fallback_reason;
    if let Some(session) = state.recording_session.lock().clone() {
        run.session_id = session.id;
        run.context = session.context.persisted_metadata();
        run.profile_id = Some(session.profile.profile_id);
        run.formatting_level = session.formatting_level;
        run.destination = session.destination;
    }
    *state.pending_failed_pipeline_run.lock() = Some(run);
}

pub(crate) fn finalize_product_result(
    state: &Arc<AppState>,
    mut result: PipelineRun,
) -> PipelineRun {
    // Output policy is part of the recording session and must be attached before
    // any output transformation. Otherwise a profile-selected Literal/Aggressive
    // policy would be applied only after Smart formatting had already run.
    if let Some(session) = state.recording_session.lock().clone() {
        result.session_id = session.id.clone();
        result.started_at_ms = session.started_at_ms;
        result.context = session.context.persisted_metadata();
        result.profile_id = Some(session.profile.profile_id);
        result.formatting_level = session.formatting_level;
        result.content_hint = session
            .profile
            .content_type
            .as_deref()
            .and_then(crate::pipeline_contract::ContentType::parse_str)
            .unwrap_or(result.content_hint);
        result.destination = session.destination;
        result.delivery.destination = session.destination;
        result.delivery.target_hwnd = session.delivery_target.hwnd;
        result.delivery.target_process_id = session.delivery_target.process_id;
        result.delivery.target_focus_id = session.delivery_target.focus_id;
    }
    synthesize_provider_attempts(state, &mut result);
    if !result.attempts.is_empty()
        && !result
            .journal
            .iter()
            .any(|stage| stage.stage == StageKind::Recognition && stage.provider.is_some())
    {
        for attempt in result
            .attempts
            .clone()
            .into_iter()
            .filter(|attempt| !attempt.id.starts_with("attempt-quick-refine"))
        {
            let mut stage = StageRecord::completed(
                StageKind::Recognition,
                attempt.duration_ms.unwrap_or_default(),
            );
            stage.id = format!("recognition-{}", attempt.id);
            stage.started_at_ms = attempt.started_at_ms;
            stage.provider = Some(attempt.provider.clone());
            stage.model = Some(attempt.model.clone());
            stage.transport = Some(attempt.transport);
            stage.usage = attempt.usage.clone();
            stage.error = attempt.error.clone();
            stage.status = match attempt.status {
                AttemptStatus::Success => crate::pipeline_run::StageStatus::Success,
                AttemptStatus::Failed => crate::pipeline_run::StageStatus::Failed,
                AttemptStatus::Skipped => crate::pipeline_run::StageStatus::Skipped,
                AttemptStatus::Running => crate::pipeline_run::StageStatus::Running,
                AttemptStatus::Pending => crate::pipeline_run::StageStatus::Pending,
            };
            result.add_stage(stage);
        }
    }
    if result.used_fallback
        && !result
            .journal
            .iter()
            .any(|stage| stage.stage == StageKind::Fallback)
    {
        let mut fallback = StageRecord::completed(StageKind::Fallback, 0);
        fallback.metadata.insert(
            "reason".into(),
            result
                .fallback_reason
                .clone()
                .unwrap_or_else(|| "provider_fallback".into())
                .into(),
        );
        result.add_stage(fallback);
    }
    // Global strict pass (may double-apply with in-mode; safe / idempotent).
    let vocab = state.vocabulary.read().clone();
    let ts = std::time::Instant::now();
    let (text, hits) = crate::vocabulary::apply_strict_literals(&result.final_text, &vocab);
    let strict_ms = ts.elapsed().as_millis() as u64;
    result.strict_literals_ms = Some(
        result
            .strict_literals_ms
            .unwrap_or(0)
            .saturating_add(strict_ms),
    );
    if !hits.is_empty() {
        log::info!("modes: strict literals applied ({})", hits.join(", "));
        result
            .stages
            .push(format!("strict_literals:{}", hits.len()));
        result.warnings.extend(hits);
    }
    let raw_text = result
        .whisper_text
        .clone()
        .or_else(|| result.deepgram_text.clone())
        .or_else(|| result.gemini_text.clone())
        .unwrap_or_else(|| result.final_text.clone());
    let refined_candidate = crate::transcription::remove_known_transcription_artifacts(&text);
    // Strict vocabulary is a deliberate user rewrite: guard against the raw text with
    // the same literals applied, or a term like "NextJS" → "Next.js" would make the
    // guard discard the whole refinement.
    let (guard_source, _) = crate::vocabulary::apply_strict_literals(&raw_text, &vocab);
    let refinement_guard =
        crate::transformations::enforce_protected_spans(&guard_source, &refined_candidate);
    result.warnings.extend(refinement_guard.warnings.clone());
    result.transcript.set_raw_once(raw_text.clone());
    result.transcript.refined = Some(refinement_guard.text.clone());

    let backtrack_started = std::time::Instant::now();
    let backtracked =
        crate::transformations::apply_backtrack(&refinement_guard.text, result.formatting_level);
    let backtrack_ms = backtrack_started.elapsed().as_millis() as u64;
    result.timings.backtrack_ms = Some(backtrack_ms);
    result.add_stage(StageRecord::completed(StageKind::Backtrack, backtrack_ms));
    result.warnings.extend(backtracked.warnings);

    crate::pipeline_run::emit_pipeline_progress(
        state,
        crate::pipeline_run::PipelineProgressEvent {
            kind: crate::pipeline_run::PipelineProgressKind::Formatting,
            message: Some("Formatando saída".into()),
            ..Default::default()
        },
    );
    let formatting_target = formatting_target_for_result(&result);
    let formatting_started = std::time::Instant::now();
    let formatted = crate::transformations::apply_smart_formatting(
        &backtracked.text,
        result.formatting_level,
        formatting_target,
    );
    let formatting_ms = formatting_started.elapsed().as_millis() as u64;
    result.timings.formatting_ms = Some(formatting_ms);
    result.add_stage(StageRecord::completed(StageKind::Formatting, formatting_ms));
    result.warnings.extend(formatted.warnings);

    let code_guard_started = std::time::Instant::now();
    // Formatting must not damage spans of its own input. Guarding against the raw
    // recognizer text here would throw away refinement, vocabulary and backtrack.
    let guarded =
        crate::transformations::enforce_protected_spans(&backtracked.text, &formatted.text);
    let code_guard_ms = code_guard_started.elapsed().as_millis() as u64;
    result.timings.code_guard_ms = Some(code_guard_ms);
    result.add_stage(StageRecord::completed(StageKind::CodeGuard, code_guard_ms));
    result.warnings.extend(guarded.warnings);
    result.transcript.formatted = Some(guarded.text.clone());
    result.final_text = guarded.text;
    result
}

fn synthesize_provider_attempts(state: &AppState, result: &mut PipelineRun) {
    if !result.attempts.is_empty() {
        return;
    }
    let started = crate::pipeline_run::epoch_ms().saturating_sub(result.transcription_latency_ms);
    let whisper_failure = result
        .stages
        .iter()
        .find_map(|stage| stage.strip_prefix("whisper_failed:"));
    if let Some(text) = result
        .whisper_text
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        result.attempts.push(completed_attempt(
            "attempt-whisper",
            "groq",
            "whisper-large-v3-turbo",
            AudioTransport::Multipart,
            result.whisper_ms.unwrap_or_default(),
            text.len(),
            0,
        ));
    } else if let Some(error) = whisper_failure {
        result.attempts.push(failed_attempt(
            "attempt-whisper",
            "groq",
            "whisper-large-v3-turbo",
            AudioTransport::Multipart,
            result.whisper_ms.unwrap_or_default(),
            "provider_error",
            error,
        ));
    }

    if result.mode != TranscriptionMode::FastAccurate {
        return;
    }
    let choice = state.gemini_pipelines.read().fast_accurate.clone();
    let provider = match choice.provider {
        GeminiProvider::GoogleAiStudio => "google-ai-studio",
        GeminiProvider::OpenRouter => "openrouter",
    };
    let model = choice.model_id().to_string();
    let transport = match result.gemini_transport.as_deref() {
        Some("multipart") => AudioTransport::Multipart,
        Some("files_api" | "resumable_file") => AudioTransport::ResumableFile,
        Some("raw_binary") => AudioTransport::RawBinary,
        Some("url") => AudioTransport::Url,
        Some("websocket_stream") => AudioTransport::WebSocketStream,
        _ => AudioTransport::InlineBase64,
    };
    if let Some(text) = result
        .gemini_text
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        let mut attempt = completed_attempt(
            "attempt-gemini",
            provider,
            &model,
            transport,
            result
                .gemini_ms
                .or(result.gemini_generate_ms)
                .unwrap_or_default(),
            text.len(),
            result.usage.bytes_sent.unwrap_or_default() as usize,
        );
        attempt.started_at_ms = started;
        attempt.usage = result.usage.clone();
        result.attempts.push(attempt);
    } else if result.used_fallback {
        let reason = result
            .fallback_reason
            .clone()
            .unwrap_or_else(|| "provider unavailable".into());
        result.attempts.push(failed_attempt(
            "attempt-gemini",
            provider,
            &model,
            transport,
            result
                .gemini_ms
                .or(result.gemini_generate_ms)
                .unwrap_or_default(),
            "provider_error",
            &reason,
        ));
    }
}

fn formatting_target_for_result(result: &PipelineRun) -> crate::transformations::FormattingTarget {
    if result.content_hint == crate::pipeline_contract::ContentType::Programming {
        return crate::transformations::FormattingTarget::Code;
    }
    let domain = result
        .context
        .domain
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let title = result
        .context
        .window_title
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ["chatgpt.com", "gemini.google.com", "claude.ai"]
        .iter()
        .any(|candidate| domain == *candidate)
        || title.contains("codex")
    {
        crate::transformations::FormattingTarget::Markdown
    } else {
        crate::transformations::FormattingTarget::PlainText
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline_contract::TranscriptionMode;

    #[test]
    fn mode_history_fields() {
        let r = PipelineRun {
            final_text: "olá mundo".into(),
            mode: TranscriptionMode::UltraFast,
            model: "whisper-large-v3-turbo".into(),
            stages: vec!["whisper".into()],
            transcription_latency_ms: 42,
            history_engine_label: "UltraFast/Whisper".into(),
            whisper_ms: Some(42),
            total_pipeline_ms: Some(42),
            whisper_text: Some("olá mundo".into()),
            ..Default::default()
        };
        let h =
            mode_result_to_history("1".into(), "2026-01-01 00:00".into(), None, 1000, "mic", &r);
        assert_eq!(h.mode.as_deref(), Some("ultra-fast"));
        assert_eq!(h.whisper_ms, Some(42));
        assert_eq!(h.sanitizer_latency_ms, None);
        assert_eq!(h.total_pipeline_ms, Some(42));
        assert!(h.debug_info.is_some());
    }

    #[test]
    fn ultra_history_keeps_sanitizer_ms() {
        let r = PipelineRun {
            final_text: "x".into(),
            mode: TranscriptionMode::UltraPrecise,
            model: "gemini-3.5-flash-lite".into(),
            stages: vec!["sanitizer:120ms".into()],
            transcription_latency_ms: 500,
            history_engine_label: "UltraPrecise".into(),
            sanitizer_ms: Some(120),
            whisper_ms: Some(80),
            gemini_generate_ms: Some(200),
            gemini_transport: Some("inline".into()),
            total_pipeline_ms: Some(500),
            ..Default::default()
        };
        let h = mode_result_to_history("2".into(), "d".into(), None, 1000, "mic", &r);
        assert_eq!(h.sanitizer_latency_ms, Some(120));
        assert_eq!(h.sanitizer_ms, Some(120));
        assert_eq!(h.gemini_transport.as_deref(), Some("inline"));
        assert_eq!(h.gemini_generate_ms, Some(200));
    }

    #[test]
    fn failed_mode_entry() {
        let h = mode_failed_history(
            "3".into(),
            "d".into(),
            None,
            0,
            "mic",
            TranscriptionMode::FastAccurate,
            "boom".into(),
        );
        assert_eq!(h.is_error, Some(true));
        assert!(h.sanitizer_latency_ms.is_none());
    }
    #[test]
    fn delivery_failure_preserves_text_and_reports_actionable_error() {
        let mut run = PipelineRun::success(
            "synthetic-run",
            TranscriptionMode::UltraFast,
            "texto preservado",
        );
        run.delivery.error = Some(crate::pipeline_run::PipelineError {
            kind: crate::pipeline_run::PipelineErrorKind::Delivery,
            code: "target_changed".into(),
            message: "Campo mudou".into(),
            retryable: false,
        });
        let entry = mode_result_to_history(
            "synthetic".into(),
            "2026-09-04".into(),
            None,
            1000,
            "mic",
            &run,
        );
        assert_eq!(entry.is_error, Some(true));
        assert_eq!(entry.text, "texto preservado");
        assert!(entry
            .error_message
            .as_deref()
            .unwrap()
            .contains("Campo mudou"));
    }

    #[test]
    fn delivery_to_changed_focus_target_records_success_without_error() {
        let mut run = PipelineRun::success(
            "switch-app-run",
            TranscriptionMode::UltraFast,
            "Texto entregue no chat",
        );
        run.delivery.target_hwnd = Some(202);
        run.delivery.target_process_id = Some(8888);
        run.delivery.paste_attempted = true;
        run.delivery.paste_ok = true;
        run.delivery.clipboard_ok = true;
        run.delivery.error = None;

        let entry = mode_result_to_history(
            "switch-app".into(),
            "2026-09-05".into(),
            None,
            1200,
            "mic",
            &run,
        );
        assert_eq!(entry.is_error, Some(false));
        assert!(entry.error_message.is_none());
        assert_eq!(entry.text, "Texto entregue no chat");
    }
}
