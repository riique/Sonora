//! Transcription product pipeline orchestration (AssemblyAI direct, ElevenLabs direct, OpenRouter STT/multimodal,
//! and automatic OpenRouter/Groq Whisper fallback on provider failure).

use std::sync::Arc;
use std::time::Duration;

use crate::models::{AppState, HistoryEntry, SanitizerDebug};
use crate::pipeline_contract::{TranscriptionMode, TranscriptionProvider};
use crate::pipeline_run::{
    AttemptResultMetadata, AttemptStatus, AudioTransport, CostKind, CostRecord, PipelineError,
    PipelineErrorKind, PipelineRun, ProviderAttempt, StageKind, StageRecord, UsageRecord,
};
use crate::transcription::telemetry::{
    compute_realtime_factor, est_throughput, est_total_tokens, log_latency,
};

fn provider_history_label(provider: TranscriptionProvider) -> &'static str {
    match provider {
        TranscriptionProvider::AssemblyAi => "AssemblyAI",
        TranscriptionProvider::OpenRouter => "OpenRouter",
        TranscriptionProvider::ElevenLabs => "ElevenLabs",
    }
}

fn provider_slug(provider: TranscriptionProvider) -> &'static str {
    match provider {
        TranscriptionProvider::AssemblyAi => "assemblyai",
        TranscriptionProvider::OpenRouter => "openrouter",
        TranscriptionProvider::ElevenLabs => "elevenlabs",
    }
}

fn adaptive_request_timeout(duration_ms: Option<u64>, audio_bytes: usize) -> Duration {
    let duration_secs = duration_ms
        .map(|ms| ms.div_ceil(1000))
        .unwrap_or_else(|| (audio_bytes as u64) / 32_000);
    let secs = (duration_secs.saturating_mul(2).saturating_add(45)).clamp(45, 300);
    Duration::from_secs(secs)
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

/// Concise system + user prompt used ONLY when a custom OpenRouter model ID
/// resolves to the multimodal chat/completions endpoint.
fn multimodal_transcription_prompts(glossary_block: &str) -> (String, String) {
    let system = "Você é um motor de transcrição fiel de áudio. O áudio é a única fonte autoritativa.\n\
Priorize português do Brasil e preserve palavras ou expressões em inglês (code-switching) sem traduzir.\n\
Não invente, complete, resuma ou parafraseie palavras. Preserve termos técnicos, comandos e identificadores.\n\
Se não houver fala humana inteligível, retorne saída vazia. Retorne somente o texto transcrito.".to_string();

    let user = if glossary_block.trim().is_empty() {
        "Transcreva fielmente o áudio anexado. Retorne somente o texto final.".to_string()
    } else {
        format!(
            "Transcreva fielmente o áudio anexado. Use o glossário abaixo apenas como referência de grafia quando o áudio sustentar o termo:\n{}\n\nRetorne somente o texto final.",
            glossary_block.trim()
        )
    };
    (system, user)
}

#[derive(Debug, Clone)]
struct ProviderSpeechOutput {
    text: String,
    model: String,
    transport: AudioTransport,
    usage: UsageRecord,
    generation_id: Option<String>,
    language: Option<String>,
    base64_ms: Option<u64>,
    upload_ms: Option<u64>,
    poll_ms: Option<u64>,
    poll_count: Option<u32>,
    generate_ms: Option<u64>,
    request_ms: Option<u64>,
    ttfb_ms: Option<u64>,
    reported_total_tokens: Option<usize>,
}

async fn run_openrouter_audio(
    state: &AppState,
    audio: &[u8],
    ext: &str,
    glossary_block: &str,
    model: &str,
    api_key: &str,
    duration_ms: Option<u64>,
) -> Result<ProviderSpeechOutput, String> {
    let adaptive_timeout = adaptive_request_timeout(duration_ms, audio.len());
    let route = crate::openrouter::detect_audio_route(model).await;
    log::info!(
        "modes: OpenRouter automatic audio route model={} endpoint={}",
        model,
        route.as_str()
    );
    let out = match route {
        crate::openrouter::OpenRouterAudioRoute::MultimodalLlm => {
            let (system_instruction, user_prompt) =
                multimodal_transcription_prompts(glossary_block);
            crate::openrouter::generate_with_audio(
                audio,
                ext,
                &system_instruction,
                &user_prompt,
                model,
                api_key,
                adaptive_timeout,
            )
            .await?
        }
        crate::openrouter::OpenRouterAudioRoute::SpeechToText => {
            let vocab = state.vocabulary.read().clone();
            let assembly_options = if crate::openrouter::is_assemblyai_model(model) {
                Some(crate::openrouter::AssemblyAiOptions {
                    prompt: Some(crate::assemblyai::context_prompt().to_string()),
                    keyterms_prompt: crate::assemblyai::build_keyterms(
                        &vocab,
                        crate::assemblyai::MAX_OPENROUTER_KEYTERMS,
                    ),
                })
            } else {
                None
            };
            let vocab_hint = if !crate::openrouter::is_assemblyai_model(model) {
                crate::vocabulary::whisper_prompt_hint(&vocab)
            } else {
                None
            };
            let keyterms = if crate::openrouter::is_elevenlabs_model(model) {
                crate::elevenlabs::build_keyterms(&vocab)
            } else {
                Vec::new()
            };
            crate::openrouter::transcribe_audio_with_assembly(
                audio,
                ext,
                model,
                api_key,
                Duration::from_secs(120).max(adaptive_timeout),
                vocab_hint.as_deref(),
                assembly_options.as_ref(),
                &keyterms,
            )
            .await?
        }
    };
    let usage = openrouter_usage(&out);
    Ok(ProviderSpeechOutput {
        text: out.text,
        model: model.to_string(),
        transport: out.transport,
        usage,
        generation_id: out.generation_id,
        language: None,
        base64_ms: (out.base64_ms > 0).then_some(out.base64_ms),
        upload_ms: None,
        poll_ms: None,
        poll_count: None,
        generate_ms: Some(out.request_ms),
        request_ms: Some(out.request_ms),
        ttfb_ms: out.ttfb_ms,
        reported_total_tokens: out.reported_total_tokens,
    })
}

async fn run_direct_assemblyai(
    state: &AppState,
    audio: &[u8],
    model: &str,
    api_key: &str,
    duration_ms: Option<u64>,
) -> Result<ProviderSpeechOutput, String> {
    let vocab = state.vocabulary.read().clone();
    let keyterms_prompt =
        crate::assemblyai::build_keyterms(&vocab, crate::assemblyai::MAX_DIRECT_KEYTERMS);
    let out = crate::assemblyai::transcribe_audio(crate::assemblyai::AssemblyAiTranscribeRequest {
        audio,
        api_key,
        model,
        duration_ms,
        keyterms_prompt,
        prompt: Some(crate::assemblyai::context_prompt()),
    })
    .await?;

    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert(
        "transport".into(),
        AudioTransport::RawBinary.as_str().into(),
    );
    metadata.insert("upload_ms".into(), out.upload_ms.into());
    metadata.insert("submit_ms".into(), out.submit_ms.into());
    metadata.insert("poll_ms".into(), out.poll_ms.into());
    metadata.insert("poll_count".into(), out.poll_count.into());
    if let Some(confidence) = out.confidence {
        metadata.insert("confidence".into(), confidence.into());
    }

    let usage = UsageRecord {
        audio_seconds: out.audio_duration,
        bytes_sent: Some(out.bytes_sent),
        cost: CostRecord::default(),
        metadata,
        ..Default::default()
    };

    Ok(ProviderSpeechOutput {
        text: out.text,
        model: out.model,
        transport: AudioTransport::RawBinary,
        usage,
        generation_id: Some(out.transcript_id),
        language: out.language_code,
        base64_ms: None,
        upload_ms: Some(out.upload_ms),
        poll_ms: Some(out.poll_ms),
        poll_count: Some(out.poll_count),
        generate_ms: Some(out.total_ms),
        request_ms: Some(out.total_ms),
        ttfb_ms: None,
        reported_total_tokens: None,
    })
}

async fn run_direct_elevenlabs(
    state: &AppState,
    audio: &[u8],
    ext: &str,
    model: &str,
    api_key: &str,
    duration_ms: Option<u64>,
) -> Result<ProviderSpeechOutput, String> {
    let keyterms = crate::elevenlabs::build_keyterms(&state.vocabulary.read());
    let out = crate::elevenlabs::transcribe_audio(crate::elevenlabs::ElevenLabsTranscribeRequest {
        audio,
        ext,
        api_key,
        model,
        keyterms,
        timeout: Duration::from_secs(120).max(adaptive_request_timeout(duration_ms, audio.len())),
    })
    .await?;

    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert(
        "transport".into(),
        AudioTransport::Multipart.as_str().into(),
    );
    metadata.insert(
        "audio_format".into(),
        if out.sent_pcm { "pcm_s16le_16" } else { "file" }.into(),
    );
    metadata.insert("keyterms".into(), out.keyterms_sent.into());

    let usage = UsageRecord {
        audio_seconds: out.audio_duration_secs,
        bytes_sent: Some(out.bytes_sent),
        cost: CostRecord::default(),
        metadata,
        ..Default::default()
    };

    Ok(ProviderSpeechOutput {
        text: out.text,
        model: out.model,
        transport: AudioTransport::Multipart,
        usage,
        generation_id: out.transcription_id,
        language: out.language_code,
        base64_ms: None,
        upload_ms: None,
        poll_ms: None,
        poll_count: None,
        generate_ms: Some(out.request_ms),
        request_ms: Some(out.request_ms),
        ttfb_ms: None,
        reported_total_tokens: None,
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
        user_parts.push(format!("[PRIMARY]\n{}", g));
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

fn apply_timing_from_provider(result: &mut PipelineRun, out: &ProviderSpeechOutput) {
    result.usage.merge(&out.usage);
    result.gemini_transport = Some(out.transport.as_str().to_string());
    result.base64_ms = out.base64_ms.or(result.base64_ms);
    result.files_upload_ms = out.upload_ms.or(result.files_upload_ms);
    result.files_poll_ms = out.poll_ms.or(result.files_poll_ms);
    result.files_poll_count = out.poll_count.or(result.files_poll_count);
    result.gemini_generate_ms = out.generate_ms.or(result.gemini_generate_ms);
    result.gemini_ms = out.request_ms.or(result.gemini_ms);
    result.timings.request_ms = out.request_ms;
    result.timings.ttfb_ms = out.ttfb_ms;
    result.openrouter_generation_id = out
        .generation_id
        .clone()
        .or_else(|| result.openrouter_generation_id.clone());
    result.reported_total_tokens = out.reported_total_tokens.or(result.reported_total_tokens);
    if let Some(u) = out.upload_ms {
        let poll = out.poll_ms.unwrap_or(0);
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

struct WhisperBackupContext<'a> {
    primary_label: &'a str,
    audio: &'a [u8],
    ext: &'a str,
    reason: &'a str,
}

/// Runs the Whisper backup after the primary audio provider failed, recording the attempt.
async fn whisper_backup(
    state: &Arc<AppState>,
    ctx: WhisperBackupContext<'_>,
    attempts: &mut Vec<ProviderAttempt>,
    result_meta: &mut PipelineRun,
    stages: &mut Vec<String>,
) -> Result<(String, String), String> {
    emit_fallback_progress(
        state,
        ctx.primary_label,
        "OpenRouter/Groq Whisper",
        ctx.reason,
    );
    let started = std::time::Instant::now();
    match run_openrouter_whisper_fallback(state, ctx.audio, ctx.ext).await {
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

enum PrimaryOutcome {
    Transcribed(String, String),
    /// The recognizer succeeded (HTTP 200) and explicitly confirmed there was
    /// no intelligible speech. Must NOT fall back to Whisper (which hallucinates
    /// text on silence).
    NoSpeech,
    Failed(String, String),
}

pub async fn run_fast_accurate(
    state: &Arc<AppState>,
    audio: Vec<u8>,
    ext: &str,
    _file_name: &str,
    duration_ms: Option<u64>,
) -> Result<PipelineRun, String> {
    let t0 = std::time::Instant::now();
    let audio_len = audio.len();
    let choice = state.transcription_pipelines.read().fast_accurate.clone();
    let model_id = choice.model_id().to_string();
    let api_key = match choice.provider {
        TranscriptionProvider::AssemblyAi => state.next_assemblyai_key(),
        TranscriptionProvider::OpenRouter => state.next_openrouter_key(),
        TranscriptionProvider::ElevenLabs => state.next_elevenlabs_key(),
    };
    let glossary_block = {
        let vocab = state.vocabulary.read().clone();
        crate::vocabulary::format_glossary_for_prompt(&vocab)
    };
    let provider_label = provider_history_label(choice.provider);
    let provider_id = provider_slug(choice.provider);
    let default_transport = match choice.provider {
        TranscriptionProvider::AssemblyAi => AudioTransport::RawBinary,
        TranscriptionProvider::OpenRouter | TranscriptionProvider::ElevenLabs => {
            AudioTransport::Multipart
        }
    };
    log::info!(
        "modes: FastAccurate → {} model={}",
        provider_label,
        model_id
    );

    let mut stages = Vec::new();
    let mut whisper_text = None;
    let mut primary_text = None;
    let mut result_meta = PipelineRun::default();
    let mut attempts = Vec::new();

    let primary = match api_key {
        None => {
            stages.push("primary_skipped_no_key".into());
            attempts.push(failed_attempt(
                "attempt-1",
                provider_id,
                &model_id,
                default_transport,
                0,
                "missing_key",
                "Chave do provedor não configurada",
            ));
            PrimaryOutcome::Failed(
                "provider_missing_api_key".into(),
                format!("Chave da API de {provider_label} ausente"),
            )
        }
        Some(key) => {
            stages.push(format!(
                "model_api:{}",
                match choice.provider {
                    TranscriptionProvider::AssemblyAi => "assemblyai-prerecorded",
                    TranscriptionProvider::OpenRouter => "openrouter-automatic",
                    TranscriptionProvider::ElevenLabs => "elevenlabs-speech-to-text",
                }
            ));
            stages.push("primary_transcribe".into());
            let provider_started = std::time::Instant::now();
            let generated = match choice.provider {
                TranscriptionProvider::AssemblyAi => {
                    run_direct_assemblyai(state, &audio, &model_id, &key, duration_ms).await
                }
                TranscriptionProvider::ElevenLabs => {
                    run_direct_elevenlabs(state, &audio, ext, &model_id, &key, duration_ms).await
                }
                TranscriptionProvider::OpenRouter => {
                    run_openrouter_audio(
                        state,
                        &audio,
                        ext,
                        &glossary_block,
                        &model_id,
                        &key,
                        duration_ms,
                    )
                    .await
                }
            };
            let provider_ms = provider_started.elapsed().as_millis() as u64;
            match generated {
                Ok(out) if !out.text.trim().is_empty() => {
                    apply_timing_from_provider(&mut result_meta, &out);
                    let mut attempt = completed_attempt(
                        "attempt-1",
                        provider_id,
                        &out.model,
                        out.transport,
                        provider_ms,
                        out.text.len(),
                        audio_len,
                    );
                    attempt.usage = out.usage.clone();
                    attempt.result.generation_id = out.generation_id.clone();
                    attempt.result.language = out.language.clone();
                    attempts.push(attempt);
                    stages.push(format!("transport:{}", out.transport.as_str()));
                    primary_text = Some(out.text.clone());
                    PrimaryOutcome::Transcribed(out.text, out.model)
                }
                Ok(out) => {
                    apply_timing_from_provider(&mut result_meta, &out);
                    let mut attempt = completed_attempt(
                        "attempt-1",
                        provider_id,
                        &out.model,
                        out.transport,
                        provider_ms,
                        0,
                        audio_len,
                    );
                    attempt.usage = out.usage.clone();
                    attempt.result.generation_id = out.generation_id.clone();
                    attempt.result.language = out.language.clone();
                    attempts.push(attempt);
                    stages.push("no_intelligible_speech".into());
                    PrimaryOutcome::NoSpeech
                }
                Err(error) => {
                    attempts.push(failed_attempt(
                        "attempt-1",
                        provider_id,
                        &model_id,
                        default_transport,
                        provider_ms,
                        "provider_error",
                        &error,
                    ));
                    PrimaryOutcome::Failed(format!("provider_error: {error}"), error)
                }
            }
        }
    };

    let mut used_fallback = false;
    let mut fallback_reason = None;
    let mut label = format!("FastAccurate/{provider_label}");
    let (final_text, model) = match primary {
        PrimaryOutcome::Transcribed(text, model) => (text, model),
        PrimaryOutcome::NoSpeech => {
            let message = "Nenhuma fala inteligível detectada na gravação.".to_string();
            remember_failed_attempts(
                state,
                TranscriptionMode::FastAccurate,
                attempts,
                &message,
                false,
                None,
            );
            return Err(message);
        }
        PrimaryOutcome::Failed(reason, message) => {
            used_fallback = true;
            fallback_reason = Some(reason);
            label = "FastAccurate/WhisperFallback/OpenRouter-Groq".into();
            match whisper_backup(
                state,
                WhisperBackupContext {
                    primary_label: provider_label,
                    audio: &audio,
                    ext,
                    reason: &message,
                },
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
        gemini_text: primary_text,
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
        openrouter_generation_id: result_meta.openrouter_generation_id,
        reported_total_tokens: result_meta.reported_total_tokens,
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
        let choice = state.transcription_pipelines.read().fast_accurate.clone();
        let (provider, transport) = match choice.provider {
            TranscriptionProvider::AssemblyAi => ("assemblyai", AudioTransport::RawBinary),
            TranscriptionProvider::OpenRouter => ("openrouter", AudioTransport::Multipart),
            TranscriptionProvider::ElevenLabs => ("elevenlabs", AudioTransport::Multipart),
        };
        attempts.push(failed_attempt(
            "attempt-1",
            provider,
            choice.model_id(),
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
    let choice = state.transcription_pipelines.read().fast_accurate.clone();
    let provider = provider_slug(choice.provider);
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
            "attempt-primary",
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
            "attempt-primary",
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
            model: "assemblyai-stt-3-7".into(),
            stages: vec!["sanitizer:120ms".into()],
            transcription_latency_ms: 500,
            history_engine_label: "FastAccurate/AssemblyAI".into(),
            sanitizer_ms: Some(120),
            whisper_ms: Some(80),
            gemini_generate_ms: Some(200),
            gemini_transport: Some("raw_binary".into()),
            total_pipeline_ms: Some(500),
            ..Default::default()
        };
        let h = mode_result_to_history("2".into(), "d".into(), None, 1000, "mic", &r);
        assert_eq!(h.sanitizer_latency_ms, Some(120));
        assert_eq!(h.sanitizer_ms, Some(120));
        assert_eq!(h.gemini_transport.as_deref(), Some("raw_binary"));
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
