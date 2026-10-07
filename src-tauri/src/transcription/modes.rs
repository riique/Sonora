//! Product modes: UltraFast, FastAccurate, Precise, UltraPrecise.

use std::sync::Arc;

use crate::gemini::{
    adaptive_generate_timeout, encode_audio_base64, fast_accurate_transcription_prompt,
    mime_for_ext, precise_refinement_prompt, refine_precise, refine_precise_with_file,
    refine_ultraprecise, spawn_cleanup, transcribe_audio, transcribe_inline, transcribe_with_file,
    transcription_prompt, ultraprecise_refinement_prompt, upload_and_wait, GeminiAudioTransport,
    GeminiGenerateResult, GeminiOperation, GeminiPrompt, GeminiStageTiming, TranscribeRequest,
    PRECISE_PROMPT_VERSION, TRANSCRIBE_PROMPT_VERSION, ULTRAPRECISE_PROMPT_VERSION,
};
use crate::models::{AppState, HistoryEntry, SanitizerDebug};
use crate::pipeline_contract::{GeminiProvider, TranscriptionMode};
use crate::pipeline_run::{
    AttemptResultMetadata, AttemptStatus, AudioTransport, CostKind, CostRecord, PipelineError,
    PipelineErrorKind, PipelineRun, ProviderAttempt, StageKind, StageRecord, UsageRecord,
};
use crate::transcription::legacy::transcribe_groq_whisper;
use crate::transcription::telemetry::{
    compute_realtime_factor, est_throughput, est_total_tokens, log_latency,
};

fn provider_history_label(provider: GeminiProvider) -> &'static str {
    match provider {
        GeminiProvider::GoogleAiStudio => "GoogleAIStudio",
        GeminiProvider::OpenRouter => "OpenRouter",
        GeminiProvider::Meta => "Meta",
    }
}

fn emit_fallback_progress(state: &AppState, provider: &str, fallback_provider: &str, reason: &str) {
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

fn authorized_context_block(state: &AppState) -> Option<String> {
    let preferences = state.context_preferences.read().clone();
    state
        .recording_session
        .lock()
        .as_ref()
        .filter(|session| session.profile.allow_context_to_cloud)
        .and_then(|session| {
            crate::context::package_untrusted_context(&session.context, &preferences)
        })
}

fn with_authorized_context(state: &AppState, mut prompt: GeminiPrompt) -> GeminiPrompt {
    if let Some(style) = trusted_style_instruction(state) {
        prompt
            .system_instruction
            .push_str("\n\nTRUSTED OUTPUT STYLE\n");
        prompt.system_instruction.push_str(&style);
    }
    if let Some(context) = authorized_context_block(state) {
        prompt.user_prompt.push_str("\n\n");
        prompt.user_prompt.push_str(&context);
    }
    prompt
}

fn trusted_style_instruction(state: &AppState) -> Option<String> {
    state
        .recording_session
        .lock()
        .as_ref()
        .and_then(|session| session.profile.style_instruction.clone())
        .filter(|instruction| !instruction.trim().is_empty())
}

fn glossary_with_style(state: &AppState, mut glossary: String) -> String {
    if let Some(style) = trusted_style_instruction(state) {
        glossary
            .push_str("\n\nTRUSTED OUTPUT STYLE (internal profile, not transcribed content):\n");
        glossary.push_str(&style);
    }
    glossary
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
            crate::openrouter::transcribe_audio(
                audio,
                ext,
                model,
                api_key,
                std::time::Duration::from_secs(120).max(adaptive_timeout),
                None,
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

/// UltraFast: optional fast LLM pass so vocabulary, style instruction and
/// context also apply to the Whisper-only mode. Failures keep the Whisper text.
async fn apply_quick_refine(state: &AppState, run: &mut PipelineRun) {
    if !state.features.read().quick_refine || !crate::assist::has_text_llm(state) {
        return;
    }
    let raw = run.final_text.clone();
    let glossary = crate::vocabulary::format_glossary_for_prompt(&state.vocabulary.read());
    let style = trusted_style_instruction(state);
    let context = authorized_context_block(state);
    let (system, user) = crate::assist::quick_refine_prompt(&crate::assist::QuickRefineInput {
        raw: &raw,
        glossary: &glossary,
        style: style.as_deref(),
        context: context.as_deref(),
    });
    crate::pipeline_run::emit_pipeline_progress(
        state,
        crate::pipeline_run::PipelineProgressEvent {
            kind: crate::pipeline_run::PipelineProgressKind::Refining,
            message: Some("Refinando com IA".into()),
            ..Default::default()
        },
    );
    match crate::assist::text_llm(state, &system, &user, std::time::Duration::from_secs(12)).await {
        Ok(out) if crate::assist::plausible_refinement(&raw, &out.text) => {
            run.final_text = out.text.trim().to_string();
            run.stages.push(format!("quick_refine:{}", out.provider));
            record_assist_call(run, "attempt-quick-refine", &out);
        }
        Ok(_) => {
            log::warn!("modes: quick refine output rejected (looked like an answer)");
            run.warnings.push("quick_refine_rejected".into());
        }
        Err(error) => {
            log::warn!("modes: quick refine failed: {error}");
            run.warnings.push("quick_refine_failed".into());
        }
    }
}

/// Last resort when every cloud provider failed: whisper.cpp on this computer.
async fn offline_fallback(
    state: &AppState,
    audio: Vec<u8>,
    mode: TranscriptionMode,
    cloud_error: &str,
) -> Option<PipelineRun> {
    let model = state.features.read().offline_model;
    emit_fallback_progress(state, "nuvem", "Whisper local", cloud_error);
    let started = std::time::Instant::now();
    match crate::local_whisper::transcribe(model, audio).await {
        Ok(text) => {
            let ms = started.elapsed().as_millis() as u64;
            let model_label = format!("whisper.cpp/{model:?}").to_lowercase();
            let reason = format!("offline: {cloud_error}");
            Some(PipelineRun {
                final_text: text.clone(),
                mode,
                model: model_label.clone(),
                whisper_text: Some(text.clone()),
                stages: vec!["offline_whisper".into()],
                history_engine_label: "offline/whisper.cpp".into(),
                used_fallback: true,
                fallback_reason: Some(reason.clone()),
                fallback: crate::pipeline_run::FallbackRecord {
                    used: true,
                    forced: false,
                    reason: Some(reason),
                    from_provider: Some("cloud".into()),
                    to_provider: Some("local".into()),
                },
                attempts: vec![completed_attempt(
                    "attempt-offline",
                    "local",
                    &model_label,
                    AudioTransport::RawBinary,
                    ms,
                    text.len(),
                    0,
                )],
                transcription_latency_ms: ms,
                total_pipeline_ms: Some(ms),
                ..Default::default()
            })
        }
        Err(error) => {
            log::warn!("modes: offline fallback failed: {error}");
            None
        }
    }
}

/// UltraFast: audio → Whisper → optional quick LLM refine (no sanitizer).
/// Provider is user-selectable: OpenRouter STT (Whisper pinned to Groq)
/// or Groq Audio Transcriptions directly. Same model variant either way.
pub async fn run_ultra_fast(
    state: &Arc<AppState>,
    audio: Vec<u8>,
    file_name: &str,
    mime: &str,
    ext: &str,
    _duration_ms: Option<u64>,
) -> Result<PipelineRun, String> {
    use crate::pipeline_contract::UltraFastProvider;

    let t0 = std::time::Instant::now();
    let (provider, whisper) = {
        let routes = state.gemini_pipelines.read();
        (routes.ultra_fast_provider, routes.ultra_fast_whisper)
    };
    match provider {
        UltraFastProvider::Groq => {
            run_ultra_fast_via_groq(state, audio, file_name, mime, whisper, t0).await
        }
        UltraFastProvider::OpenRouter => {
            run_ultra_fast_via_openrouter(state, audio, ext, whisper, t0).await
        }
    }
}

async fn run_ultra_fast_via_openrouter(
    state: &Arc<AppState>,
    audio: Vec<u8>,
    ext: &str,
    whisper: crate::pipeline_contract::OpenRouterWhisperModel,
    t0: std::time::Instant,
) -> Result<PipelineRun, String> {
    let model = whisper.openrouter_id();
    let api_key = state.next_openrouter_key().ok_or_else(|| {
        "Configure uma chave do OpenRouter em Provedores e APIs para usar o modo Ultrarrápido."
            .to_string()
    })?;
    log::info!(
        "modes: UltraFast → OpenRouter STT model={} provider=groq (sanitizer off)",
        model
    );
    let vocabulary_hint = crate::vocabulary::whisper_prompt_hint(&state.vocabulary.read());
    let generated = crate::openrouter::transcribe_audio(
        &audio,
        ext,
        model,
        &api_key,
        std::time::Duration::from_secs(120),
        vocabulary_hint.as_deref(),
    )
    .await?;
    let text = generated.text;

    if text.trim().is_empty() {
        return Err("Nenhum texto detectado na gravação.".to_string());
    }

    let ms = t0.elapsed().as_millis() as u64;
    let attempt = ProviderAttempt {
        id: "attempt-1".into(),
        provider: "openrouter/groq".into(),
        model: model.into(),
        transport: generated.transport,
        started_at_ms: crate::pipeline_run::epoch_ms().saturating_sub(ms),
        duration_ms: Some(generated.request_ms),
        status: AttemptStatus::Success,
        usage: UsageRecord {
            audio_seconds: generated.reported_audio_seconds,
            input_tokens: generated.reported_input_tokens.map(|value| value as u64),
            output_tokens: generated.reported_output_tokens.map(|value| value as u64),
            total_tokens: generated.reported_total_tokens.map(|value| value as u64),
            bytes_sent: Some(generated.bytes_sent),
            cost: generated
                .reported_cost_usd
                .map_or_else(CostRecord::default, |amount| CostRecord {
                    kind: CostKind::Actual,
                    amount_usd: Some(amount),
                    source: Some("openrouter_response_usage".into()),
                }),
            ..Default::default()
        },
        result: AttemptResultMetadata {
            generation_id: generated.generation_id.clone(),
            output_chars: Some(text.trim().len()),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut run = PipelineRun {
        final_text: text.trim().to_string(),
        mode: TranscriptionMode::UltraFast,
        model: model.into(),
        stages: vec![
            "openrouter_stt".into(),
            "provider:groq".into(),
            "model_api:speech-to-text".into(),
        ],
        whisper_text: Some(text.trim().to_string()),
        transcription_latency_ms: ms,
        history_engine_label: "UltraFast/OpenRouter/Groq".into(),
        whisper_ms: Some(generated.request_ms),
        base64_ms: Some(generated.base64_ms),
        openrouter_generation_id: generated.generation_id,
        reported_total_tokens: generated.reported_total_tokens,
        total_pipeline_ms: Some(ms),
        attempts: vec![attempt],
        journal: vec![StageRecord::completed(StageKind::Recognition, ms)],
        timings: crate::pipeline_run::PipelineTimings {
            request_ms: Some(generated.request_ms),
            ttfb_ms: generated.ttfb_ms,
            provider_ms: Some(generated.request_ms),
            ..Default::default()
        },
        ..Default::default()
    };
    apply_quick_refine(state, &mut run).await;
    Ok(run)
}

async fn run_ultra_fast_via_groq(
    state: &Arc<AppState>,
    audio: Vec<u8>,
    file_name: &str,
    mime: &str,
    whisper: crate::pipeline_contract::OpenRouterWhisperModel,
    t0: std::time::Instant,
) -> Result<PipelineRun, String> {
    let model = whisper.groq_id();
    let api_key = state.next_groq_key().ok_or_else(|| {
        "Configure uma chave da Groq em Provedores e APIs para usar o modo Ultrarrápido."
            .to_string()
    })?;
    log::info!(
        "modes: UltraFast → Groq STT model={} (sanitizer off)",
        model
    );
    let bytes_sent = audio.len() as u64;
    let vocabulary_hint = crate::vocabulary::whisper_prompt_hint(&state.vocabulary.read());
    let text = crate::groq::call_whisper_api_with_model(
        audio,
        file_name,
        mime,
        model,
        &api_key,
        vocabulary_hint.as_deref(),
    )
    .await
    .map_err(|e| crate::transcription::fallback::groq_err_to_message(&e))?;

    if text.trim().is_empty() {
        return Err("Nenhum texto detectado na gravação.".to_string());
    }

    let ms = t0.elapsed().as_millis() as u64;
    let attempt = ProviderAttempt {
        id: "attempt-1".into(),
        provider: "groq".into(),
        model: model.into(),
        transport: AudioTransport::Multipart,
        started_at_ms: crate::pipeline_run::epoch_ms().saturating_sub(ms),
        duration_ms: Some(ms),
        status: AttemptStatus::Success,
        usage: UsageRecord {
            bytes_sent: Some(bytes_sent),
            ..Default::default()
        },
        result: AttemptResultMetadata {
            output_chars: Some(text.trim().len()),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut run = PipelineRun {
        final_text: text.trim().to_string(),
        mode: TranscriptionMode::UltraFast,
        model: model.into(),
        stages: vec![
            "whisper".into(),
            "provider:groq".into(),
            "model_api:speech-to-text".into(),
        ],
        whisper_text: Some(text.trim().to_string()),
        transcription_latency_ms: ms,
        history_engine_label: "UltraFast/Groq".into(),
        whisper_ms: Some(ms),
        total_pipeline_ms: Some(ms),
        attempts: vec![attempt],
        journal: vec![StageRecord::completed(StageKind::Recognition, ms)],
        timings: crate::pipeline_run::PipelineTimings {
            request_ms: Some(ms),
            provider_ms: Some(ms),
            ..Default::default()
        },
        ..Default::default()
    };
    apply_quick_refine(state, &mut run).await;
    Ok(run)
}

/// FastAccurate: Gemini (inline or Files) + glossary + strict literals.
pub async fn run_fast_accurate(
    state: &Arc<AppState>,
    audio: Vec<u8>,
    ext: &str,
    file_name: &str,
    _mime: &str,
    duration_ms: Option<u64>,
) -> Result<PipelineRun, String> {
    let t0 = std::time::Instant::now();
    let audio_len = audio.len();
    let choice = state.gemini_pipelines.read().fast_accurate.clone();
    let model_id = choice.resolved_model_id()?;
    let fallback = *state.gemini_fallback_to_whisper.read();
    let api_key = match choice.provider {
        GeminiProvider::GoogleAiStudio => state.next_google_key(),
        GeminiProvider::OpenRouter => state.next_openrouter_key(),
        GeminiProvider::Meta => state.next_meta_key(),
    };
    let context_block = authorized_context_block(state);
    let glossary_block = {
        let vocab = state.vocabulary.read().clone();
        glossary_with_style(state, crate::vocabulary::format_glossary_for_prompt(&vocab))
    };
    let file_tagging_enabled = *state.file_tagging_enabled.read();
    let provider_label = provider_history_label(choice.provider);
    log::info!(
        "modes: FastAccurate → {} model={} (fallback_to_whisper={})",
        provider_label,
        model_id,
        fallback
    );

    let mut stages = Vec::new();
    let mut used_fallback = false;
    let mut fallback_reason = None;
    let mut whisper_text = None;
    let mut gemini_text = None;
    let mut label = format!("FastAccurate/{provider_label}");
    let mut result_meta = PipelineRun::default();
    let mut attempts = Vec::new();
    let gemini_provider = match choice.provider {
        GeminiProvider::GoogleAiStudio => "google-ai-studio",
        GeminiProvider::OpenRouter => "openrouter",
        GeminiProvider::Meta => "meta",
    };

    let (final_text, model) = match api_key {
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
            if !fallback {
                let message =
                    "Configure uma chave para o provedor selecionado em Provedores e APIs.";
                remember_failed_attempts(
                    state,
                    TranscriptionMode::FastAccurate,
                    attempts,
                    message,
                    false,
                    None,
                );
                return Err(message.to_string());
            }
            used_fallback = true;
            fallback_reason = Some("gemini_missing_api_key".into());
            emit_fallback_progress(
                state,
                "Gemini",
                "OpenRouter/Groq Whisper",
                "Chave do provedor selecionado ausente",
            );
            label = "FastAccurate/WhisperFallback/OpenRouter-Groq".into();
            let tw = std::time::Instant::now();
            let w_result = run_openrouter_whisper_fallback(state, &audio, ext).await;
            let w = match w_result {
                Ok(outcome) => {
                    let whisper_ms = tw.elapsed().as_millis() as u64;
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
                    whisper_text = Some(outcome.text.clone());
                    (outcome.text, outcome.model.to_string())
                }
                Err(error) => {
                    let whisper_ms = tw.elapsed().as_millis() as u64;
                    attempts.push(failed_attempt(
                        "attempt-2",
                        "openrouter/groq",
                        state
                            .gemini_pipelines
                            .read()
                            .ultra_fast_whisper
                            .openrouter_id(),
                        AudioTransport::Multipart,
                        whisper_ms,
                        "provider_error",
                        &error,
                    ));
                    let message =
                        format!("Gemini indisponível e o fallback Whisper falhou: {error}");
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
            };
            w
        }
        Some(key) => {
            stages.push(format!(
                "model_api:{}",
                if choice.provider == GeminiProvider::Meta {
                    "meta-asr"
                } else if choice.provider == GeminiProvider::OpenRouter {
                    "automatic"
                } else if crate::gemini::is_transcribe_model(&model_id) {
                    "google-interactions"
                } else {
                    "google-generate-content"
                }
            ));
            if choice.provider == GeminiProvider::Meta {
                stages.push("meta_transcribe".into());
            } else {
                stages.push("gemini_transcribe".into());
            }
            let custom_vocabulary: Vec<String> = {
                let vocab = state.vocabulary.read();
                vocab
                    .iter()
                    .filter(|t| t.enabled)
                    .flat_map(|t| std::iter::once(t.canonical.clone()).chain(t.aliases.clone()))
                    .take(1000)
                    .collect()
            };
            let req = TranscribeRequest {
                audio_bytes: audio.clone(),
                ext: ext.to_string(),
                api_key: key.clone(),
                model: model_id.clone(),
                display_name: file_name.to_string(),
                duration_ms,
                glossary_block,
                file_tagging_enabled,
                custom_vocabulary: custom_vocabulary.clone(),
                untrusted_context: context_block.clone(),
            };
            let provider_started = std::time::Instant::now();
            let generated = if choice.provider == GeminiProvider::Meta {
                let language_bias = if choice.meta_languages.is_empty() {
                    Some(vec!["Portuguese".to_string(), "English".to_string()])
                } else {
                    Some(choice.meta_languages.clone())
                };
                let meta_res = crate::meta_asr::transcribe(
                    &audio,
                    &key,
                    Some(&model_id),
                    (!custom_vocabulary.is_empty()).then_some(custom_vocabulary),
                    language_bias,
                )
                .await;
                match meta_res {
                    Ok(outcome) => Ok(GeminiGenerateResult {
                        operation: GeminiOperation::Transcribe,
                        text: outcome.text,
                        model: outcome.model,
                        prompt_version: "meta-asr-1.0".to_string(),
                        latency_ms: outcome.latency_ms,
                        remote_file_name: None,
                        transport: None,
                        timing: GeminiStageTiming {
                            base64_ms: None,
                            files_upload_ms: None,
                            files_poll_ms: None,
                            files_poll_count: None,
                            generate_ms: Some(outcome.latency_ms),
                            delete_ms: None,
                        },
                        usage: UsageRecord {
                            bytes_sent: Some(outcome.bytes_sent as u64),
                            ..Default::default()
                        },
                    }),
                    Err(e) => Err(e),
                }
            } else if choice.provider == GeminiProvider::OpenRouter {
                let prompt = with_authorized_context(
                    state,
                    fast_accurate_transcription_prompt(
                        &req.glossary_block,
                        req.file_tagging_enabled,
                    ),
                );
                openrouter_audio_result(crate::transcription::modes::OpenrouterAudioResultInput {
                    audio: &req.audio_bytes,
                    ext,
                    prompt: &prompt,
                    model: &model_id,
                    api_key: &req.api_key,
                    duration_ms,
                    operation: GeminiOperation::Transcribe,
                    prompt_version: TRANSCRIBE_PROMPT_VERSION,
                })
                .await
            } else {
                transcribe_audio(req).await
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
                    (r.text, r.model)
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
                    if !fallback {
                        remember_failed_attempts(
                            state,
                            TranscriptionMode::FastAccurate,
                            attempts,
                            "O Gemini não retornou texto.",
                            false,
                            None,
                        );
                        return Err("O Gemini não retornou texto.".to_string());
                    }
                    used_fallback = true;
                    fallback_reason = Some("gemini_empty".into());
                    emit_fallback_progress(
                        state,
                        "Gemini",
                        "OpenRouter/Groq Whisper",
                        "Resposta vazia do provedor selecionado",
                    );
                    label = "FastAccurate/WhisperFallback/OpenRouter-Groq".into();
                    let tw = std::time::Instant::now();
                    let w_result = run_openrouter_whisper_fallback(state, &audio, ext).await;
                    let w = match w_result {
                        Ok(outcome) => {
                            let whisper_ms = tw.elapsed().as_millis() as u64;
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
                            whisper_text = Some(outcome.text.clone());
                            (outcome.text, outcome.model.to_string())
                        }
                        Err(error) => {
                            let whisper_ms = tw.elapsed().as_millis() as u64;
                            attempts.push(failed_attempt(
                                "attempt-2",
                                "openrouter/groq",
                                state
                                    .gemini_pipelines
                                    .read()
                                    .ultra_fast_whisper
                                    .openrouter_id(),
                                AudioTransport::Multipart,
                                whisper_ms,
                                "provider_error",
                                &error,
                            ));
                            let message = format!(
                                "Gemini retornou vazio e o fallback Whisper falhou: {error}"
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
                    };
                    w
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
                    if !fallback {
                        let message = format!("Gemini: {e}");
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
                    used_fallback = true;
                    fallback_reason = Some(format!("gemini_error: {}", e));
                    emit_fallback_progress(state, "Gemini", "OpenRouter/Groq Whisper", &e);
                    label = "FastAccurate/WhisperFallback/OpenRouter-Groq".into();
                    let tw = std::time::Instant::now();
                    let w_result = run_openrouter_whisper_fallback(state, &audio, ext).await;
                    let w = match w_result {
                        Ok(outcome) => {
                            let whisper_ms = tw.elapsed().as_millis() as u64;
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
                            whisper_text = Some(outcome.text.clone());
                            (outcome.text, outcome.model.to_string())
                        }
                        Err(whisper_error) => {
                            let whisper_ms = tw.elapsed().as_millis() as u64;
                            attempts.push(failed_attempt(
                                "attempt-2",
                                "openrouter/groq",
                                state
                                    .gemini_pipelines
                                    .read()
                                    .ultra_fast_whisper
                                    .openrouter_id(),
                                AudioTransport::Multipart,
                                whisper_ms,
                                "provider_error",
                                &whisper_error,
                            ));
                            let message = format!("Gemini falhou ({e}) e o fallback Whisper também falhou: {whisper_error}");
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
                    };
                    w
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

/// Precise: Whisper ∥ (Base64 or upload) → Gemini refine.
pub async fn run_precise(
    state: &Arc<AppState>,
    audio: Vec<u8>,
    ext: &str,
    file_name: &str,
    mime: &str,
    duration_ms: Option<u64>,
) -> Result<PipelineRun, String> {
    let t0 = std::time::Instant::now();
    let choice = state.gemini_pipelines.read().precise.clone();
    let model_id = choice.resolved_model_id()?;
    let api_key = match choice.provider {
        GeminiProvider::GoogleAiStudio => state.next_google_key(),
        GeminiProvider::OpenRouter => state.next_openrouter_key(),
        GeminiProvider::Meta => state.next_meta_key(),
    };
    let Some(api_key) = api_key else {
        return Err(
            "Configure uma chave para o provedor selecionado em Provedores e APIs.".to_string(),
        );
    };
    let context_block = authorized_context_block(state);
    let glossary_block = {
        let vocab = state.vocabulary.read().clone();
        let mut block =
            glossary_with_style(state, crate::vocabulary::format_glossary_for_prompt(&vocab));
        if let Some(context) = context_block.as_deref() {
            block.push_str("\n\n");
            block.push_str(context);
        }
        block
    };
    let vocab_snapshot = state.vocabulary.read().clone();
    let file_tagging_enabled = *state.file_tagging_enabled.read();
    let display = if file_name.trim().is_empty() {
        format!("sonora-precise.{}", ext)
    } else {
        file_name.to_string()
    };
    let mime_g = mime_for_ext(ext);
    let transport = if choice.provider == GeminiProvider::OpenRouter {
        GeminiAudioTransport::Inline
    } else {
        crate::gemini::select_gemini_audio_transport(
            audio.len(),
            duration_ms.or_else(|| crate::gemini::estimate_wav_duration_ms(&audio)),
            mime_g,
        )?
    };

    log::info!(
        "modes: Precise → Whisper ∥ prep, transport={:?}",
        transport.as_str()
    );

    let whisper_audio = audio.clone();
    let state_w = state.clone();
    let file_name_w = file_name.to_string();
    let mime_w = mime.to_string();
    let whisper_fut = async move {
        let tw = std::time::Instant::now();
        let r = transcribe_groq_whisper(&state_w, whisper_audio, &file_name_w, &mime_w).await;
        (r, tw.elapsed().as_millis() as u64)
    };

    let prep_audio = audio.clone();
    let key_u = api_key.clone();
    let display_u = display.clone();
    let prep_fut = async move {
        match transport {
            GeminiAudioTransport::Inline => {
                let (b64, ms) = encode_audio_base64(&prep_audio);
                PrepOutcome::Inline { b64, base64_ms: ms }
            }
            GeminiAudioTransport::FilesApi => {
                let tu = std::time::Instant::now();
                match upload_and_wait(&key_u, &prep_audio, mime_g, &display_u).await {
                    Ok((guard, timing)) => PrepOutcome::Files {
                        guard,
                        timing,
                        wall_ms: tu.elapsed().as_millis() as u64,
                    },
                    Err(e) => PrepOutcome::FilesErr(e),
                }
            }
        }
    };

    let ((whisper_res, whisper_ms), prep) = tokio::join!(whisper_fut, prep_fut);
    let mut stages = vec![
        format!("whisper_parallel:{}ms", whisper_ms),
        format!("transport:{}", transport.as_str()),
        format!(
            "model_api:{}",
            if choice.provider == GeminiProvider::OpenRouter {
                "automatic"
            } else {
                "google-generate-content"
            }
        ),
    ];
    let mut meta = PipelineRun {
        whisper_ms: Some(whisper_ms),
        gemini_transport: Some(transport.as_str().into()),
        ..Default::default()
    };

    match (whisper_res, prep) {
        (Ok(w), PrepOutcome::Inline { b64, base64_ms }) => {
            meta.base64_ms = Some(base64_ms);
            stages.push(format!("base64:{}ms", base64_ms));
            let w_trim = w.trim().to_string();
            if w_trim.is_empty() {
                let pure = if choice.provider == GeminiProvider::OpenRouter {
                    let prompt =
                        with_authorized_context(state, transcription_prompt(file_tagging_enabled));
                    openrouter_audio_result(
                        crate::transcription::modes::OpenrouterAudioResultInput {
                            audio: &audio,
                            ext,
                            prompt: &prompt,
                            model: &model_id,
                            api_key: &api_key,
                            duration_ms,
                            operation: GeminiOperation::Transcribe,
                            prompt_version: TRANSCRIBE_PROMPT_VERSION,
                        },
                    )
                    .await
                } else {
                    transcribe_inline(
                        &api_key,
                        &model_id,
                        &audio,
                        mime_g,
                        file_tagging_enabled,
                        context_block.as_deref(),
                        Some((b64, base64_ms)),
                    )
                    .await
                };
                return finish_precise_pure(crate::transcription::modes::FinishPrecisePureInput {
                    pure,
                    t0,
                    stages,
                    meta,
                    whisper_ms,
                    upload_ms: None,
                    reason: "whisper_empty_gemini_pure",
                    vocab: &vocab_snapshot,
                });
            }
            let refined = if choice.provider == GeminiProvider::OpenRouter {
                let prompt =
                    precise_refinement_prompt(&w_trim, &glossary_block, file_tagging_enabled);
                openrouter_audio_result(crate::transcription::modes::OpenrouterAudioResultInput {
                    audio: &audio,
                    ext,
                    prompt: &prompt,
                    model: &model_id,
                    api_key: &api_key,
                    duration_ms,
                    operation: GeminiOperation::Refine,
                    prompt_version: PRECISE_PROMPT_VERSION,
                })
                .await
            } else {
                refine_precise(crate::gemini::refinement::RefinePreciseInput {
                    api_key: &api_key,
                    model: &model_id,
                    audio: &audio,
                    ext,
                    display_name: &display,
                    whisper_hypothesis: &w_trim,
                    glossary_block: &glossary_block,
                    file_tagging_enabled,
                    duration_ms,
                    precomputed_b64: Some((b64, base64_ms)),
                })
                .await
            };
            finish_precise_refine(
                refined,
                t0,
                stages,
                meta,
                w_trim,
                whisper_ms,
                &vocab_snapshot,
            )
        }
        (
            Ok(w),
            PrepOutcome::Files {
                guard,
                timing,
                wall_ms,
            },
        ) => {
            meta.files_upload_ms = Some(timing.upload_ms);
            meta.files_poll_ms = Some(timing.poll_ms);
            meta.files_poll_count = Some(timing.poll_count);
            meta.upload_ms = Some(wall_ms);
            stages.push(format!("upload_parallel:{}ms", wall_ms));
            let w_trim = w.trim().to_string();
            if w_trim.is_empty() {
                stages.push("whisper_empty".into());
                let file_ref = guard.file_ref();
                let pure = transcribe_with_file(
                    &api_key,
                    &model_id,
                    &file_ref,
                    file_tagging_enabled,
                    context_block.as_deref(),
                    duration_ms,
                    audio.len(),
                )
                .await;
                spawn_cleanup(guard);
                return finish_precise_pure(crate::transcription::modes::FinishPrecisePureInput {
                    pure,
                    t0,
                    stages,
                    meta,
                    whisper_ms,
                    upload_ms: Some(wall_ms),
                    reason: "whisper_empty_gemini_pure",
                    vocab: &vocab_snapshot,
                });
            }
            let file_ref = guard.file_ref();
            let refined =
                refine_precise_with_file(crate::gemini::refinement::RefinePreciseWithFileInput {
                    api_key: &api_key,
                    model: &model_id,
                    file: &file_ref,
                    whisper_hypothesis: &w_trim,
                    glossary_block: &glossary_block,
                    file_tagging_enabled,
                    duration_ms,
                    audio_bytes: audio.len(),
                })
                .await;
            spawn_cleanup(guard);
            finish_precise_refine(
                refined,
                t0,
                stages,
                meta,
                w_trim,
                whisper_ms,
                &vocab_snapshot,
            )
        }
        (Ok(w), PrepOutcome::FilesErr(up_err)) => {
            stages.push("upload_failed".into());
            let w_trim = w.trim().to_string();
            if w_trim.is_empty() {
                return Err(format!(
                    "Upload Gemini falhou ({}) e o Whisper não retornou texto.",
                    up_err
                ));
            }
            let ts = std::time::Instant::now();
            let (final_text, hits) =
                crate::vocabulary::apply_strict_literals(&w_trim, &vocab_snapshot);
            meta.strict_literals_ms = Some(ts.elapsed().as_millis() as u64);
            if !hits.is_empty() {
                stages.push(format!("strict_literals:{}", hits.len()));
            }
            let total = t0.elapsed().as_millis() as u64;
            Ok(PipelineRun {
                final_text,
                mode: TranscriptionMode::Precise,
                model: "whisper-large-v3-turbo".into(),
                stages,
                used_fallback: true,
                fallback_reason: Some(format!("upload_failed: {}", up_err)),
                whisper_text: Some(w_trim),
                transcription_latency_ms: total,
                history_engine_label: "Precise/WhisperOnly".into(),
                whisper_ms: Some(whisper_ms),
                total_pipeline_ms: Some(total),
                gemini_transport: meta.gemini_transport,
                warnings: vec![format!("upload_failed: {}", up_err)],
                ..meta
            })
        }
        (Err(w_err), PrepOutcome::Inline { b64, base64_ms }) => {
            meta.base64_ms = Some(base64_ms);
            stages.push(format!("whisper_failed:{}", w_err));
            let pure = if choice.provider == GeminiProvider::OpenRouter {
                let prompt =
                    with_authorized_context(state, transcription_prompt(file_tagging_enabled));
                openrouter_audio_result(crate::transcription::modes::OpenrouterAudioResultInput {
                    audio: &audio,
                    ext,
                    prompt: &prompt,
                    model: &model_id,
                    api_key: &api_key,
                    duration_ms,
                    operation: GeminiOperation::Transcribe,
                    prompt_version: TRANSCRIBE_PROMPT_VERSION,
                })
                .await
            } else {
                transcribe_inline(
                    &api_key,
                    &model_id,
                    &audio,
                    mime_g,
                    file_tagging_enabled,
                    context_block.as_deref(),
                    Some((b64, base64_ms)),
                )
                .await
            };
            finish_precise_pure(crate::transcription::modes::FinishPrecisePureInput {
                pure,
                t0,
                stages,
                meta,
                whisper_ms,
                upload_ms: None,
                reason: &format!("whisper_failed: {}", w_err),
                vocab: &vocab_snapshot,
            })
        }
        (
            Err(w_err),
            PrepOutcome::Files {
                guard,
                timing,
                wall_ms,
            },
        ) => {
            meta.files_upload_ms = Some(timing.upload_ms);
            meta.files_poll_ms = Some(timing.poll_ms);
            meta.files_poll_count = Some(timing.poll_count);
            meta.upload_ms = Some(wall_ms);
            stages.push(format!("whisper_failed:{}", w_err));
            let file_ref = guard.file_ref();
            let pure = transcribe_with_file(
                &api_key,
                &model_id,
                &file_ref,
                file_tagging_enabled,
                context_block.as_deref(),
                duration_ms,
                audio.len(),
            )
            .await;
            spawn_cleanup(guard);
            finish_precise_pure(crate::transcription::modes::FinishPrecisePureInput {
                pure,
                t0,
                stages,
                meta,
                whisper_ms,
                upload_ms: Some(wall_ms),
                reason: &format!("whisper_failed: {}", w_err),
                vocab: &vocab_snapshot,
            })
        }
        (Err(w_err), PrepOutcome::FilesErr(up_err)) => Err(format!(
            "Ambos falharam no modo Preciso:\n• Whisper: {}\n• Upload Gemini: {}",
            w_err, up_err
        )),
    }
}

enum PrepOutcome {
    Inline {
        b64: String,
        base64_ms: u64,
    },
    Files {
        guard: crate::gemini::RemoteFileGuard,
        timing: crate::gemini::UploadTiming,
        wall_ms: u64,
    },
    FilesErr(String),
}

fn finish_precise_refine(
    refined: Result<crate::gemini::GeminiGenerateResult, String>,
    t0: std::time::Instant,
    mut stages: Vec<String>,
    mut meta: PipelineRun,
    w_trim: String,
    whisper_ms: u64,
    vocab: &[crate::vocabulary::VocabularyTerm],
) -> Result<PipelineRun, String> {
    match refined {
        Ok(r) if !r.text.trim().is_empty() => {
            apply_timing_from_gemini(&mut meta, &r);
            if let Some(g) = r.timing.generate_ms {
                stages.push(format!("gemini_refine:{}ms", g));
            }
            let ts = std::time::Instant::now();
            let (final_text, hits) = crate::vocabulary::apply_strict_literals(r.text.trim(), vocab);
            meta.strict_literals_ms = Some(ts.elapsed().as_millis() as u64);
            if !hits.is_empty() {
                stages.push(format!("strict_literals:{}", hits.len()));
            }
            let total = t0.elapsed().as_millis() as u64;
            Ok(PipelineRun {
                final_text,
                mode: TranscriptionMode::Precise,
                model: r.model,
                stages,
                whisper_text: Some(w_trim),
                gemini_text: Some(r.text.trim().to_string()),
                transcription_latency_ms: total,
                history_engine_label: "Precise/Whisper+Gemini".into(),
                whisper_ms: Some(whisper_ms),
                total_pipeline_ms: Some(total),
                ..meta
            })
        }
        Ok(_) | Err(_) => {
            let reason = match &refined {
                Ok(_) => "gemini_refine_empty".to_string(),
                Err(e) => format!("gemini_refine_error: {}", e),
            };
            log::warn!(
                "modes: Precise refine failed ({}); delivering Whisper",
                reason
            );
            stages.push("whisper_deliver".into());
            let ts = std::time::Instant::now();
            let (final_text, hits) = crate::vocabulary::apply_strict_literals(&w_trim, vocab);
            meta.strict_literals_ms = Some(ts.elapsed().as_millis() as u64);
            if !hits.is_empty() {
                stages.push(format!("strict_literals:{}", hits.len()));
            }
            let total = t0.elapsed().as_millis() as u64;
            Ok(PipelineRun {
                final_text,
                mode: TranscriptionMode::Precise,
                model: "whisper-large-v3-turbo".into(),
                stages,
                used_fallback: true,
                fallback_reason: Some(reason.clone()),
                whisper_text: Some(w_trim),
                transcription_latency_ms: total,
                history_engine_label: "Precise/WhisperFallback".into(),
                whisper_ms: Some(whisper_ms),
                total_pipeline_ms: Some(total),
                warnings: vec![reason],
                ..meta
            })
        }
    }
}
pub struct FinishPrecisePureInput<'a> {
    pub pure: Result<crate::gemini::GeminiGenerateResult, String>,
    pub t0: std::time::Instant,
    pub stages: Vec<String>,
    pub meta: PipelineRun,
    pub whisper_ms: u64,
    pub upload_ms: Option<u64>,
    pub reason: &'a str,
    pub vocab: &'a [crate::vocabulary::VocabularyTerm],
}

fn finish_precise_pure(input: FinishPrecisePureInput<'_>) -> Result<PipelineRun, String> {
    let FinishPrecisePureInput {
        pure,
        t0,
        mut stages,
        mut meta,
        whisper_ms,
        upload_ms,
        reason,
        vocab,
    } = input;
    match pure {
        Ok(r) if !r.text.trim().is_empty() => {
            apply_timing_from_gemini(&mut meta, &r);
            stages.push("gemini_pure".into());
            let ts = std::time::Instant::now();
            let (final_text, hits) = crate::vocabulary::apply_strict_literals(r.text.trim(), vocab);
            meta.strict_literals_ms = Some(ts.elapsed().as_millis() as u64);
            if !hits.is_empty() {
                stages.push(format!("strict_literals:{}", hits.len()));
            }
            let total = t0.elapsed().as_millis() as u64;
            Ok(PipelineRun {
                final_text,
                mode: TranscriptionMode::Precise,
                model: r.model,
                stages,
                used_fallback: true,
                fallback_reason: Some(reason.into()),
                gemini_text: Some(r.text.trim().to_string()),
                transcription_latency_ms: total,
                history_engine_label: "Precise/GeminiPure".into(),
                whisper_ms: Some(whisper_ms),
                upload_ms,
                total_pipeline_ms: Some(total),
                warnings: vec![reason.into()],
                ..meta
            })
        }
        Ok(_) => Err(format!(
            "Whisper indisponível e o Gemini não retornou texto ({})",
            reason
        )),
        Err(ge) => Err(format!(
            "Ambos falharam no modo Preciso:\n• Whisper path: {}\n• Gemini: {}",
            reason, ge
        )),
    }
}

/// UltraPrecise: Whisper → sanitizer ∥ prep → Gemini.
pub async fn run_ultra_precise(
    state: &Arc<AppState>,
    audio: Vec<u8>,
    ext: &str,
    file_name: &str,
    mime: &str,
    duration_ms: Option<u64>,
) -> Result<PipelineRun, String> {
    let t0 = std::time::Instant::now();
    let choice = state.gemini_pipelines.read().ultra_precise.clone();
    let model_id = choice.resolved_model_id()?;
    let api_key = match choice.provider {
        GeminiProvider::GoogleAiStudio => state.next_google_key(),
        GeminiProvider::OpenRouter => state.next_openrouter_key(),
        GeminiProvider::Meta => state.next_meta_key(),
    };
    let context_block = authorized_context_block(state);
    let glossary_block = {
        let vocab = state.vocabulary.read().clone();
        let mut block =
            glossary_with_style(state, crate::vocabulary::format_glossary_for_prompt(&vocab));
        if let Some(context) = context_block.as_deref() {
            block.push_str("\n\n");
            block.push_str(context);
        }
        block
    };
    let vocab_snapshot = state.vocabulary.read().clone();
    let file_tagging_enabled = *state.file_tagging_enabled.read();
    let display = if file_name.trim().is_empty() {
        format!("sonora-ultraprecise.{}", ext)
    } else {
        file_name.to_string()
    };
    let mime_g = mime_for_ext(ext);
    let transport = if choice.provider == GeminiProvider::OpenRouter {
        GeminiAudioTransport::Inline
    } else {
        crate::gemini::select_gemini_audio_transport(
            audio.len(),
            duration_ms.or_else(|| crate::gemini::estimate_wav_duration_ms(&audio)),
            mime_g,
        )?
    };

    log::info!(
        "modes: UltraPrecise → Whisper→sanitizer ∥ prep ({})",
        transport.as_str()
    );

    // Branch A: Whisper then sanitizer (sequential chain).
    let whisper_audio = audio.clone();
    let state_w = state.clone();
    let file_name_w = file_name.to_string();
    let mime_w = mime.to_string();
    let chain_fut = async move {
        let tw = std::time::Instant::now();
        let wr = transcribe_groq_whisper(&state_w, whisper_audio, &file_name_w, &mime_w).await;
        let whisper_ms = tw.elapsed().as_millis() as u64;
        match wr {
            Ok(w) if !w.trim().is_empty() => {
                let w = w.trim().to_string();
                let ts = std::time::Instant::now();
                let sanitize = crate::transcription::run_sanitize(&state_w, &w).await;
                let sanitizer_ms = ts.elapsed().as_millis() as u64;
                Ok((w, sanitize, whisper_ms, sanitizer_ms))
            }
            Ok(_) => Err(("empty".into(), whisper_ms)),
            Err(e) => Err((e, whisper_ms)),
        }
    };

    // Branch B: Base64 or Files upload (parallel with Whisper→sanitizer).
    let prep_audio = audio.clone();
    let key_u = api_key.clone();
    let display_u = display.clone();
    let prep_fut = async move {
        match transport {
            GeminiAudioTransport::Inline => {
                let (b64, ms) = encode_audio_base64(&prep_audio);
                PrepOutcome::Inline { b64, base64_ms: ms }
            }
            GeminiAudioTransport::FilesApi => match key_u {
                Some(ref k) => match upload_and_wait(k, &prep_audio, mime_g, &display_u).await {
                    Ok((guard, timing)) => PrepOutcome::Files {
                        guard,
                        timing,
                        wall_ms: timing.upload_ms + timing.poll_ms,
                    },
                    Err(e) => PrepOutcome::FilesErr(e),
                },
                None => PrepOutcome::FilesErr("missing google key".into()),
            },
        }
    };

    let (chain_res, prep) = tokio::join!(chain_fut, prep_fut);

    let (whisper_text, sanitize, whisper_ms, sanitizer_ms) = match chain_res {
        Ok(v) => v,
        Err((e, whisper_ms)) => {
            if let PrepOutcome::Files { guard, .. } = prep {
                spawn_cleanup(guard);
            }
            if e == "empty" {
                return Err("Nenhum texto detectado na gravação (Whisper vazio).".into());
            }
            return Err(format!(
                "Whisper falhou no Ultrapreciso: {} ({}ms)",
                e, whisper_ms
            ));
        }
    };

    let mut stages = vec![
        format!("whisper:{}ms", whisper_ms),
        format!("sanitizer:{}ms", sanitizer_ms),
        format!("transport:{}", transport.as_str()),
        format!(
            "model_api:{}",
            if choice.provider == GeminiProvider::OpenRouter {
                "automatic"
            } else {
                "google-generate-content"
            }
        ),
    ];
    if sanitize.used_raw_fallback {
        stages.push("sanitizer_raw_fallback".into());
    }
    for w in &sanitize.warnings {
        stages.push(format!("sanitizer_warn:{}", w));
    }

    let sanitized_text = if sanitize.final_text.trim().is_empty() {
        whisper_text.clone()
    } else {
        sanitize.final_text.clone()
    };
    let mut meta = PipelineRun {
        whisper_ms: Some(whisper_ms),
        sanitizer_ms: Some(sanitizer_ms),
        gemini_transport: Some(transport.as_str().into()),
        debug_info: sanitize.debug_info.clone(),
        warnings: sanitize.warnings.clone(),
        ..Default::default()
    };

    let (final_text, model, used_fallback, fallback_reason, gemini_text, label) =
        match (api_key.as_ref(), prep) {
            (Some(key), PrepOutcome::Inline { b64, base64_ms }) => {
                meta.base64_ms = Some(base64_ms);
                stages.push(format!("base64:{}ms", base64_ms));
                let refined = if choice.provider == GeminiProvider::OpenRouter {
                    let prompt = ultraprecise_refinement_prompt(
                        &whisper_text,
                        &sanitized_text,
                        &glossary_block,
                        file_tagging_enabled,
                    );
                    openrouter_audio_result(
                        crate::transcription::modes::OpenrouterAudioResultInput {
                            audio: &audio,
                            ext,
                            prompt: &prompt,
                            model: &model_id,
                            api_key: key,
                            duration_ms,
                            operation: GeminiOperation::Refine,
                            prompt_version: ULTRAPRECISE_PROMPT_VERSION,
                        },
                    )
                    .await
                } else {
                    refine_ultraprecise(crate::gemini::refinement::RefineUltrapreciseInput {
                        api_key: key,
                        model: &model_id,
                        audio: &audio,
                        ext,
                        display_name: &display,
                        whisper_raw: &whisper_text,
                        sanitized: &sanitized_text,
                        glossary_block: &glossary_block,
                        file_tagging_enabled,
                        duration_ms,
                        precomputed_b64: Some((b64, base64_ms)),
                    })
                    .await
                };
                match refined {
                    Ok(r) if !r.text.trim().is_empty() => {
                        apply_timing_from_gemini(&mut meta, &r);
                        if let Some(g) = r.timing.generate_ms {
                            stages.push(format!("gemini_ultraprecise:{}ms", g));
                        }
                        (
                            r.text.trim().to_string(),
                            r.model,
                            false,
                            None,
                            Some(r.text.trim().to_string()),
                            "UltraPrecise/Whisper+Sanitizer+Gemini",
                        )
                    }
                    Ok(_) | Err(_) => {
                        let reason = match &refined {
                            Ok(_) => "gemini_empty".to_string(),
                            Err(e) => format!("gemini_error: {}", e),
                        };
                        stages.push("gemini_failed_use_sanitized".into());
                        meta.warnings.push(reason.clone());
                        (
                            sanitized_text.clone(),
                            if sanitize.used_raw_fallback {
                                "whisper-large-v3-turbo".into()
                            } else {
                                "sanitizer+whisper".into()
                            },
                            true,
                            Some(reason),
                            None,
                            "UltraPrecise/SanitizedFallback",
                        )
                    }
                }
            }
            (
                Some(key),
                PrepOutcome::Files {
                    guard,
                    timing,
                    wall_ms,
                },
            ) => {
                meta.files_upload_ms = Some(timing.upload_ms);
                meta.files_poll_ms = Some(timing.poll_ms);
                meta.files_poll_count = Some(timing.poll_count);
                meta.upload_ms = Some(wall_ms);
                stages.push(format!("upload_parallel:{}ms", wall_ms));
                let file_ref = guard.file_ref();
                let refined = crate::gemini::refine_ultraprecise_with_file(
                    crate::gemini::refinement::RefineUltrapreciseWithFileInput {
                        api_key: key,
                        model: &model_id,
                        file: &file_ref,
                        whisper_raw: &whisper_text,
                        sanitized: &sanitized_text,
                        glossary_block: &glossary_block,
                        file_tagging_enabled,
                        duration_ms,
                        audio_bytes: audio.len(),
                    },
                )
                .await;
                spawn_cleanup(guard);
                match refined {
                    Ok(r) if !r.text.trim().is_empty() => {
                        apply_timing_from_gemini(&mut meta, &r);
                        if let Some(g) = r.timing.generate_ms {
                            stages.push(format!("gemini_ultraprecise:{}ms", g));
                        }
                        (
                            r.text.trim().to_string(),
                            r.model,
                            false,
                            None,
                            Some(r.text.trim().to_string()),
                            "UltraPrecise/Whisper+Sanitizer+Gemini",
                        )
                    }
                    Ok(_) | Err(_) => {
                        let reason = match &refined {
                            Ok(_) => "gemini_empty".to_string(),
                            Err(e) => format!("gemini_error: {}", e),
                        };
                        stages.push("gemini_failed_use_sanitized".into());
                        meta.warnings.push(reason.clone());
                        (
                            sanitized_text.clone(),
                            "sanitizer+whisper".into(),
                            true,
                            Some(reason),
                            None,
                            "UltraPrecise/SanitizedFallback",
                        )
                    }
                }
            }
            (None, prep) => {
                if let PrepOutcome::Files { guard, .. } = prep {
                    spawn_cleanup(guard);
                }
                stages.push("gemini_missing_key".into());
                meta.warnings.push("gemini_missing_api_key".into());
                (
                    sanitized_text.clone(),
                    "sanitizer+whisper".into(),
                    true,
                    Some("gemini_missing_api_key".into()),
                    None,
                    "UltraPrecise/NoGemini",
                )
            }
            (_, PrepOutcome::FilesErr(e)) => {
                stages.push("upload_failed".into());
                meta.warnings.push(format!("upload_failed: {}", e));
                (
                    sanitized_text.clone(),
                    "sanitizer+whisper".into(),
                    true,
                    Some(format!("upload_failed: {}", e)),
                    None,
                    "UltraPrecise/NoGemini",
                )
            }
        };

    if final_text.trim().is_empty() {
        return Err("Falha total no Ultrapreciso: sem texto final.".into());
    }

    let ts = std::time::Instant::now();
    let (final_text, hits) =
        crate::vocabulary::apply_strict_literals(final_text.trim(), &vocab_snapshot);
    meta.strict_literals_ms = Some(ts.elapsed().as_millis() as u64);
    if !hits.is_empty() {
        stages.push(format!("strict_literals:{}", hits.len()));
    }

    let total = t0.elapsed().as_millis() as u64;
    let san_text = if sanitize.used_raw_fallback {
        None
    } else {
        Some(sanitized_text)
    };
    Ok(PipelineRun {
        final_text,
        mode: TranscriptionMode::UltraPrecise,
        model,
        stages,
        used_fallback,
        fallback_reason,
        whisper_text: Some(whisper_text),
        gemini_text,
        sanitizer_text: san_text,
        transcription_latency_ms: total,
        history_engine_label: label.into(),
        total_pipeline_ms: Some(total),
        debug_info: meta.debug_info,
        whisper_ms: meta.whisper_ms,
        sanitizer_ms: meta.sanitizer_ms,
        base64_ms: meta.base64_ms,
        upload_ms: meta.upload_ms,
        files_upload_ms: meta.files_upload_ms,
        files_poll_ms: meta.files_poll_ms,
        files_poll_count: meta.files_poll_count,
        gemini_generate_ms: meta.gemini_generate_ms,
        gemini_ms: meta.gemini_generate_ms,
        gemini_transport: meta.gemini_transport,
        strict_literals_ms: meta.strict_literals_ms,
        warnings: meta.warnings,
        usage: meta.usage,
        timings: meta.timings,
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

pub async fn run_product_mode(
    state: &Arc<AppState>,
    audio: Vec<u8>,
    file_name: &str,
    mime: &str,
    ext: &str,
) -> Result<PipelineRun, String> {
    run_product_mode_with_duration(state, audio, file_name, mime, ext, None).await
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
    let mode = *state.transcription_mode.read();
    let offline_audio = (ext.eq_ignore_ascii_case("wav")
        && state.features.read().offline_fallback
        && crate::local_whisper::is_installed(state.features.read().offline_model))
    .then(|| audio.clone());
    let result = match mode {
        TranscriptionMode::UltraFast => {
            run_ultra_fast(state, audio, file_name, mime, ext, duration_ms).await
        }
        TranscriptionMode::FastAccurate => {
            run_fast_accurate(state, audio, ext, file_name, mime, duration_ms).await
        }
        TranscriptionMode::Precise => {
            run_precise(state, audio, ext, file_name, mime, duration_ms).await
        }
        TranscriptionMode::UltraPrecise => {
            run_ultra_precise(state, audio, ext, file_name, mime, duration_ms).await
        }
    };
    let result = match result {
        Ok(result) => Ok(result),
        Err(error) => match offline_audio {
            Some(audio) => offline_fallback(state, audio, mode, &error)
                .await
                .ok_or(error),
            None => Err(error),
        },
    };
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
        let (provider, model, transport) = match mode {
            TranscriptionMode::UltraFast => match state.gemini_pipelines.read().ultra_fast_provider
            {
                crate::pipeline_contract::UltraFastProvider::Groq => {
                    ("groq", "whisper-large-v3", AudioTransport::Multipart)
                }
                crate::pipeline_contract::UltraFastProvider::OpenRouter => {
                    ("openrouter", "whisper-large-v3", AudioTransport::Multipart)
                }
            },
            TranscriptionMode::FastAccurate => {
                ("gemini", "configured-model", AudioTransport::InlineBase64)
            }
            TranscriptionMode::Precise | TranscriptionMode::UltraPrecise => {
                ("pipeline", "multi-provider", AudioTransport::Multipart)
            }
        };
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
    if result.sanitizer_ms.is_some()
        && !result
            .journal
            .iter()
            .any(|stage| stage.stage == StageKind::Sanitizer)
    {
        let mut sanitizer = StageRecord::completed(
            StageKind::Sanitizer,
            result.sanitizer_ms.unwrap_or_default(),
        );
        sanitizer.provider = Some("groq".into());
        sanitizer.model = Some(state.sanitizer.read().api_model_id().into());
        result.add_stage(sanitizer);
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
    let snippet_started = std::time::Instant::now();
    let (final_text, snippet_id) = crate::snippets::resolve(
        &guarded.text,
        &crate::snippets::list().unwrap_or_else(|error| {
            log::warn!("snippets unavailable: {error}");
            Vec::new()
        }),
    )
    .map(|(expansion, id)| (expansion, Some(id)))
    .unwrap_or_else(|| (guarded.text, None));
    let snippet_ms = snippet_started.elapsed().as_millis() as u64;
    result.timings.snippet_ms = Some(snippet_ms);
    let mut snippet_stage = StageRecord::completed(StageKind::SnippetResolution, snippet_ms);
    if let Some(snippet_id) = snippet_id {
        result.stages.push(format!("snippet:{snippet_id}"));
        snippet_stage
            .metadata
            .insert("snippet_id".into(), snippet_id.into());
    }
    result.add_stage(snippet_stage);
    result.final_text = final_text;
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

    let choice = match result.mode {
        TranscriptionMode::FastAccurate => state.gemini_pipelines.read().fast_accurate.clone(),
        TranscriptionMode::Precise => state.gemini_pipelines.read().precise.clone(),
        TranscriptionMode::UltraPrecise => state.gemini_pipelines.read().ultra_precise.clone(),
        TranscriptionMode::UltraFast => return,
    };
    let provider = match choice.provider {
        GeminiProvider::GoogleAiStudio => "google-ai-studio",
        GeminiProvider::OpenRouter => "openrouter",
        GeminiProvider::Meta => "meta",
    };
    let model = choice
        .resolved_model_id()
        .unwrap_or_else(|_| result.model.clone());
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
