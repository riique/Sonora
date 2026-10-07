//! Groq Whisper transcription client.
//!
//! Sends the in-memory WAV buffer (produced by [`crate::audio`]) to
//! the Groq Audio Transcriptions endpoint and returns the raw text
//! returned by the `whisper-large-v3-turbo` model.
//!
//! All network IO is fully async (Tokio) so the Tauri main thread is
//! never blocked while the request is in flight.

use reqwest::multipart;
use serde::{Deserialize, Serialize};
use std::{sync::OnceLock, time::Duration};

/// Official Groq endpoint for OpenAI-compatible audio transcriptions.
const GROQ_TRANSCRIPTIONS_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";

/// Official Groq endpoint for OpenAI-compatible chat completions.
const GROQ_CHAT_COMPLETIONS_URL: &str = "https://api.groq.com/openai/v1/chat/completions";

/// Sentinel string returned by the sanitizer LLM when it wants the
/// caller to abort the cleanup pass and fall back to the raw
/// Whisper transcription. Must match exactly, byte for byte.
pub const FALLBACK_RETRY_SENTINEL: &str = "[FALLBACK_RETRY]";

/// Model identifier used for every transcription request. The turbo
/// variant offers the lowest latency on the Groq LPU fleet.
const WHISPER_MODEL: &str = "whisper-large-v3-turbo";

/// Soft timeout for the HTTP exchange. The Groq API typically
/// answers a short clip in well under five seconds, so thirty
/// seconds gives plenty of headroom for longer recordings without
/// letting the request hang forever on a degraded network.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn http_client() -> Result<&'static reqwest::Client, reqwest::Error> {
    if let Some(client) = HTTP_CLIENT.get() {
        return Ok(client);
    }
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()?;
    let _ = HTTP_CLIENT.set(client);
    Ok(HTTP_CLIENT
        .get()
        .expect("HTTP client must be initialized after set"))
}

/// Errors that can surface while talking to the Groq API. Every
/// variant is graceful: the caller propagates the message to the
/// UI/log layer without crashing the Tauri process.
#[derive(Debug, thiserror::Error)]
pub enum GroqNetworkError {
    #[error("network request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("groq api returned non-success status {status}: {body}")]
    ApiError { status: u16, body: String },
    #[error("failed to parse groq response json: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("response did not contain a 'text' field")]
    MissingText,
    #[error("api key is missing or empty")]
    MissingApiKey,
}

/// Minimal deserialization target for the Groq transcription
/// response. The API may include extra fields (segments, language,
/// etc.) but only `text` is required for Phase 3.
#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: Option<String>,
}

/// Builds the multipart form, attaches the Bearer token and fires
/// the POST request to the Groq transcription endpoint.
///
/// `wav_bytes` must be a complete, self-contained RIFF/WAVE blob
/// (44-byte header followed by little-endian 16-bit PCM samples).
/// The function does not validate the buffer internally; the API
/// itself will reject malformed audio with a 4xx response that is
/// then surfaced via [`GroqNetworkError::ApiError`].
pub async fn call_whisper_api(
    audio_bytes: Vec<u8>,
    file_name: &str,
    mime: &str,
    api_key: &str,
) -> Result<String, GroqNetworkError> {
    call_whisper_api_with_model(audio_bytes, file_name, mime, WHISPER_MODEL, api_key, None).await
}

/// Same as [`call_whisper_api`] but with an explicit Whisper model id
/// (e.g. `whisper-large-v3-turbo` or `whisper-large-v3`).
/// Used by the UltraFast pipeline when the user selects Groq direct,
/// keeping the model choice in sync with the OpenRouter route. `prompt`
/// carries the vocabulary hint (Whisper biases spelling toward it).
pub async fn call_whisper_api_with_model(
    audio_bytes: Vec<u8>,
    file_name: &str,
    mime: &str,
    model: &str,
    api_key: &str,
    prompt: Option<&str>,
) -> Result<String, GroqNetworkError> {
    if api_key.trim().is_empty() {
        return Err(GroqNetworkError::MissingApiKey);
    }

    let client = http_client()?;

    // The file part must carry a filename and the correct MIME type
    // so the Groq backend dispatches to the Whisper decoder rather
    // than treating the payload as generic binary. Microphone captures
    // pass `audio.wav` / `audio/wav`; uploads pass the real filename and
    // detected MIME so Groq can decode mp3/m4a/flac/etc. directly.
    let file_part = multipart::Part::bytes(audio_bytes)
        .file_name(file_name.to_string())
        .mime_str(mime)?;

    let mut form = multipart::Form::new()
        .text("model", model.to_string())
        // Same language hint the OpenRouter route sends, so switching
        // providers does not change transcription language behavior.
        .text("language", "pt")
        .part("file", file_part);
    if let Some(prompt) = prompt.filter(|prompt| !prompt.trim().is_empty()) {
        form = form.text("prompt", prompt.to_string());
    }

    let response = client
        .post(GROQ_TRANSCRIPTIONS_URL)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await?;

    let status = response.status().as_u16();

    if status == 200 {
        let parsed: TranscriptionResponse = response.json().await?;
        parsed
            .text
            .ok_or(GroqNetworkError::MissingText)
            .map(|t| t.trim().to_string())
    } else {
        let body = response.text().await.unwrap_or_default();
        Err(GroqNetworkError::ApiError { status, body })
    }
}

/// Chat message payload sent to the Groq Chat Completions endpoint.
/// Only `role` and `content` are required for the sanitizer flow.
#[derive(Debug, Clone, Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

/// Soft cap on sanitizer completion tokens (keeps latency bounded).
const SANITIZER_MAX_TOKENS: u32 = 2048;

/// Request body for the Chat Completions call.
#[derive(Debug, Serialize)]
struct ChatCompletionsRequest {
    model: String,
    temperature: f32,
    max_tokens: u32,
    messages: Vec<ChatMessage>,
    /// Native Groq reasoning control. Only set for models that support it
    /// (the GPT-OSS family); skipped otherwise so the API does not reject it.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    /// Ask Groq to return the model's reasoning trace in a separate field so
    /// developer mode can display exactly what reasoning was used. Only set
    /// when reasoning is actually applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    include_reasoning: Option<bool>,
}

/// Minimal deserialization target for the Chat Completions response.
/// The Groq payload contains many fields but only the first choice's
/// message content is needed by the sanitizer pipeline.
#[derive(Debug, Deserialize)]
struct ChatCompletionsResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: Option<String>,
    /// Reasoning trace, present when `include_reasoning` was requested on a
    /// reasoning-capable model. Used only by developer mode.
    #[serde(default)]
    reasoning: Option<String>,
}

/// Maximum number of characters of the reasoning trace persisted in the
/// developer-mode debug snapshot. Reasoning can be long; this keeps the
/// `history.json` file from ballooning while still showing the gist.
const MAX_REASONING_TRACE_CHARS: usize = 8_000;

/// Truncates `s` to at most `max` chars on a UTF-8 boundary, appending an
/// ellipsis when something was cut.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Result of a sanitizer call: the outcome of the request plus a developer-mode
/// snapshot of exactly what was sent and received. The debug snapshot is always
/// produced (even on failure) so the Histórico can surface the request and the
/// error when developer mode is on.
pub struct SanitizerOutcome {
    /// Final plain text only (never raw JSON).
    pub result: Result<String, GroqNetworkError>,
    pub debug: crate::models::SanitizerDebug,
    /// True when structured JSON said the text was modified.
    pub changed: bool,
    /// Warnings from the model or from our parser/fallback path.
    pub warnings: Vec<String>,
    /// True when we discarded model output and will use acoustic raw.
    pub used_raw_fallback: bool,
}

/// Calls the Groq Chat Completions endpoint with `temperature = 0.0`
/// to force deterministic decoding, passing the user-edited system
/// prompt and the raw Whisper transcription as the user message.
///
/// When `reasoning_enabled` and `reasoning_supported` are both true, Groq's
/// native `reasoning_effort` parameter is sent (the GPT-OSS family accepts
/// `low`/`medium`/`high`); otherwise it is omitted so the API does not reject
/// the call. The function always returns a [`SanitizerOutcome`] carrying both
/// the result and a [`crate::models::SanitizerDebug`] capture of the request.
pub struct CallSanitizerApiInput<'a> {
    pub whisper_text: &'a str,
    pub model: &'a str,
    pub system_prompt: &'a str,
    pub untrusted_context: Option<&'a str>,
    pub glossary_block: &'a str,
    pub api_key: &'a str,
    pub reasoning_enabled: bool,
    pub reasoning_effort: &'a str,
    pub reasoning_supported: bool,
}

pub async fn call_sanitizer_api(input: CallSanitizerApiInput<'_>) -> SanitizerOutcome {
    let CallSanitizerApiInput {
        whisper_text,
        model,
        system_prompt,
        untrusted_context,
        glossary_block,
        api_key,
        reasoning_enabled,
        reasoning_effort,
        reasoning_supported,
    } = input;
    // Assemble the final system prompt: the base prompt (which may already carry
    // the dual-engine instruction appended by the caller), followed by the
    // user's personal glossary. The reasoning level is now controlled by the
    // native `reasoning_effort` request parameter rather than a prompt note.
    let mut final_system_prompt = system_prompt.to_string();

    if !glossary_block.trim().is_empty() {
        final_system_prompt.push_str(&format!(
            "\n\n--- GLOSSÁRIO PESSOAL DO USUÁRIO (PRIORIDADE ALTA) ---\n\
As entradas abaixo foram cadastradas pelo usuário. Cada linha traz o canônico, \
categoria, aliases opcionais e [LITERAL] quando o termo é rígido.\n\
Quando um trecho transcrito for claramente uma corrupção de um canônico/alias e o \
contexto encaixar, use a grafia canônica. Termos [LITERAL] têm prioridade máxima: \
nunca altere a grafia canônica e prefira-a a qualquer variante. Seja conservador: \
na dúvida, mantenha o original e NÃO force termos onde não pertencem.\n{}",
            glossary_block.trim()
        ));
    }

    let mut user_message_content = format!("[WHISPER_RAW]: {}", whisper_text);
    if let Some(context) = untrusted_context.filter(|value| !value.trim().is_empty()) {
        user_message_content.push_str("\n\n");
        user_message_content.push_str(context);
    }

    // Reasoning is only really applied when the user enabled it *and* the model
    // can honour the native parameter. This is the single source of truth the
    // debug snapshot reports back to the UI.
    let apply_reasoning = reasoning_enabled && reasoning_supported;

    let body = ChatCompletionsRequest {
        model: model.to_string(),
        temperature: 0.0,
        max_tokens: SANITIZER_MAX_TOKENS,
        messages: vec![
            ChatMessage {
                role: "system",
                content: final_system_prompt.clone(),
            },
            ChatMessage {
                role: "user",
                content: user_message_content.clone(),
            },
        ],
        // Reasoning stays off unless the user explicitly enables it (default: off).
        reasoning_effort: apply_reasoning.then(|| reasoning_effort.to_string()),
        include_reasoning: apply_reasoning.then_some(true),
    };

    // Build the debug snapshot up-front so it is available on every return path.
    let mut debug = crate::models::SanitizerDebug {
        endpoint: GROQ_CHAT_COMPLETIONS_URL.to_string(),
        model: model.to_string(),
        temperature: body.temperature,
        reasoning_enabled,
        reasoning_effort: reasoning_effort.to_string(),
        reasoning_effort_applied: apply_reasoning,
        reasoning_supported_by_model: reasoning_supported,
        system_prompt: final_system_prompt,
        user_message: user_message_content,
        request_json: serde_json::to_string_pretty(&body)
            .unwrap_or_else(|e| format!("<falha ao serializar request: {}>", e)),
        ..Default::default()
    };

    let fail = |debug: crate::models::SanitizerDebug, err: GroqNetworkError| SanitizerOutcome {
        result: Err(err),
        debug,
        changed: false,
        warnings: Vec::new(),
        used_raw_fallback: false,
    };

    if api_key.trim().is_empty() {
        debug.error = Some("api key is missing or empty".to_string());
        return fail(debug, GroqNetworkError::MissingApiKey);
    }

    let client = match http_client() {
        Ok(c) => c,
        Err(e) => {
            debug.error = Some(e.to_string());
            return fail(debug, e.into());
        }
    };

    let response = match client
        .post(GROQ_CHAT_COMPLETIONS_URL)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            debug.error = Some(e.to_string());
            return fail(debug, e.into());
        }
    };

    let status = response.status().as_u16();
    debug.response_status = Some(status);

    if status == 200 {
        let parsed: ChatCompletionsResponse = match response.json().await {
            Ok(p) => p,
            Err(e) => {
                debug.error = Some(e.to_string());
                return fail(debug, e.into());
            }
        };
        let message = parsed.choices.into_iter().next().map(|c| c.message);
        if let Some(reasoning) = message.as_ref().and_then(|m| m.reasoning.clone()) {
            debug.response_reasoning = Some(truncate_chars(&reasoning, MAX_REASONING_TRACE_CHARS));
        }
        match message.and_then(|m| m.content) {
            Some(content) => {
                let trimmed = content.trim().to_string();
                // Keep raw model payload for debug only — never surface as final text.
                debug.response_content = Some(trimmed.clone());

                if trimmed == FALLBACK_RETRY_SENTINEL {
                    return SanitizerOutcome {
                        result: Ok(FALLBACK_RETRY_SENTINEL.to_string()),
                        debug,
                        changed: false,
                        warnings: vec!["fallback_retry_sentinel".into()],
                        used_raw_fallback: true,
                    };
                }

                match crate::sanitizer_json::parse_sanitizer_content(&trimmed) {
                    Ok(structured) => SanitizerOutcome {
                        result: Ok(structured.text),
                        debug,
                        changed: structured.changed,
                        warnings: structured.warnings,
                        used_raw_fallback: false,
                    },
                    Err(parse_err) => {
                        // Parsing failed: never surface JSON/prose as final text.
                        // Empty Ok + used_raw_fallback lets the pipeline deliver acoustic raw.
                        debug.error = Some(format!("sanitizer_json: {}", parse_err));
                        log::warn!("sanitizer: structured parse failed: {}", parse_err);
                        SanitizerOutcome {
                            result: Ok(String::new()),
                            debug,
                            changed: false,
                            warnings: vec![format!("sanitizer_parse_failed: {}", parse_err)],
                            used_raw_fallback: true,
                        }
                    }
                }
            }
            None => {
                debug.error = Some("response did not contain a 'text' field".to_string());
                fail(debug, GroqNetworkError::MissingText)
            }
        }
    } else {
        let error_body = response.text().await.unwrap_or_default();
        debug.error = Some(error_body.clone());
        fail(
            debug,
            GroqNetworkError::ApiError {
                status,
                body: error_body,
            },
        )
    }
}
