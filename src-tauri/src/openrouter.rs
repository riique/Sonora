//! OpenRouter audio & text client.
//!
//! - Dedicated STT models (`/api/v1/audio/transcriptions`) use multipart/form-data.
//! - Multimodal audio LLMs (`/api/v1/chat/completions`) use Base64 `input_audio`.

use crate::pipeline_run::AudioTransport;
use base64::{engine::general_purpose, Engine as _};
use reqwest::header::{HeaderMap, HeaderValue, REFERER};
use reqwest::multipart;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::OnceLock,
    time::{Duration, Instant},
};

const CHAT_COMPLETIONS_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const TRANSCRIPTIONS_URL: &str = "https://openrouter.ai/api/v1/audio/transcriptions";
const STT_MODELS_URL: &str = "https://openrouter.ai/api/v1/models?output_modalities=transcription";
const APP_REFERER: &str = "https://github.com/riique/Sonora";
const APP_TITLE: &str = "Sonora";
static STT_MODEL_IDS: tokio::sync::OnceCell<HashSet<String>> = tokio::sync::OnceCell::const_new();
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn http_client() -> Result<&'static reqwest::Client, String> {
    if let Some(client) = HTTP_CLIENT.get() {
        return Ok(client);
    }
    let client = reqwest::Client::builder()
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())?;
    let _ = HTTP_CLIENT.set(client);
    HTTP_CLIENT
        .get()
        .ok_or_else(|| "OpenRouter HTTP client unavailable".into())
}

/// Opens (or refreshes) the pooled TLS connection so the dictation request
/// does not pay the handshake. The response status is irrelevant.
pub async fn warm_connection() {
    let Ok(client) = http_client() else { return };
    let request = client.head("https://openrouter.ai/api/v1/models").send();
    if let Ok(Ok(response)) = tokio::time::timeout(Duration::from_secs(5), request).await {
        let _ = response.bytes().await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRouterAudioRoute {
    MultimodalLlm,
    SpeechToText,
}

impl OpenRouterAudioRoute {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MultimodalLlm => "multimodal-llm",
            Self::SpeechToText => "speech-to-text",
        }
    }
}

fn app_attribution_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(REFERER, HeaderValue::from_static(APP_REFERER));
    headers.insert("X-OpenRouter-Title", HeaderValue::from_static(APP_TITLE));
    headers
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct Message<'a> {
    role: &'static str,
    content: MessageContent<'a>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum MessageContent<'a> {
    Text(&'a str),
    Parts(Vec<ContentPart<'a>>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ContentPart<'a> {
    #[serde(rename = "text")]
    Text { text: &'a str },
    #[serde(rename = "input_audio")]
    InputAudio { input_audio: InputAudio<'a> },
}

#[derive(Debug, Serialize)]
struct InputAudio<'a> {
    data: &'a str,
    format: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: String,
}

#[derive(Debug, Serialize)]
struct ProviderPreferences<'a> {
    only: [&'a str; 1],
    allow_fallbacks: bool,
    /// Provider passthrough, keyed by provider slug (e.g. Groq's Whisper `prompt`).
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct ApiUsage {
    #[serde(default)]
    total_tokens: Option<usize>,
    #[serde(default, alias = "prompt_tokens")]
    input_tokens: Option<usize>,
    #[serde(default, alias = "completion_tokens")]
    output_tokens: Option<usize>,
    #[serde(default)]
    seconds: Option<f64>,
    #[serde(default)]
    cost: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ModelCatalogResponse {
    #[serde(default)]
    data: Vec<ModelCatalogEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelCatalogEntry {
    id: String,
}

#[derive(Debug, Clone)]
pub struct OpenRouterGenerateResult {
    pub text: String,
    pub base64_ms: u64,
    pub request_ms: u64,
    pub ttfb_ms: Option<u64>,
    pub generation_id: Option<String>,
    pub reported_total_tokens: Option<usize>,
    pub reported_input_tokens: Option<usize>,
    pub reported_output_tokens: Option<usize>,
    pub reported_audio_seconds: Option<f64>,
    /// OpenRouter returns this value in the response usage object; when
    /// present it is an actual provider-reported charge, never an estimate.
    pub reported_cost_usd: Option<f64>,
    pub transport: AudioTransport,
    pub bytes_sent: u64,
}

#[derive(Debug, Clone)]
pub struct OpenRouterTextResult {
    pub text: String,
    pub request_ms: u64,
    pub ttfb_ms: Option<u64>,
    pub generation_id: Option<String>,
    pub reported_total_tokens: Option<usize>,
    pub reported_input_tokens: Option<usize>,
    pub reported_output_tokens: Option<usize>,
    pub reported_cost_usd: Option<f64>,
    pub bytes_sent: u64,
}

/// Returns true for ElevenLabs Scribe models routed via OpenRouter.
pub fn is_elevenlabs_model(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("elevenlabs/")
}

/// Dictation defaults forwarded to ElevenLabs through OpenRouter's provider
/// passthrough. OpenRouter silently drops keys a provider does not accept.
fn elevenlabs_provider_options() -> serde_json::Value {
    serde_json::json!({
        "options": {
            "elevenlabs": {
                "tag_audio_events": false,
                "no_verbatim": true,
                "timestamps_granularity": "none"
            }
        }
    })
}

/// Returns true for AssemblyAI STT models routed via OpenRouter.
pub fn is_assemblyai_model(model: &str) -> bool {
    let id = model.trim().to_ascii_lowercase();
    id.starts_with("assemblyai/")
}

fn model_looks_dedicated_stt(model: &str) -> bool {
    let id = model.trim().to_ascii_lowercase();
    id.contains("whisper")
        || id.contains("transcribe")
        || id.contains("chirp")
        || id.starts_with("assemblyai/")
        || id.starts_with("deepgram/")
        || id.starts_with("elevenlabs/")
}

fn whisper_provider_preferences(
    model: &str,
    prompt: Option<&str>,
) -> Option<ProviderPreferences<'static>> {
    model
        .to_ascii_lowercase()
        .contains("whisper")
        .then(|| ProviderPreferences {
            only: ["groq"],
            allow_fallbacks: false,
            // OpenRouter ignores a top-level `prompt` for STT; Groq receives it only
            // through provider options, where it biases spelling of known terms.
            options: prompt
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty())
                .map(|prompt| serde_json::json!({ "groq": { "prompt": prompt } })),
        })
}

async fn fetch_stt_model_ids() -> Result<HashSet<String>, String> {
    let response = tokio::time::timeout(
        Duration::from_secs(10),
        http_client()?.get(STT_MODELS_URL).send(),
    )
    .await
    .map_err(|_| "timeout ao consultar o catálogo de modelos STT".to_string())?
    .map_err(|e| format!("falha ao consultar o catálogo de modelos STT: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "catálogo de modelos STT retornou status {}",
            status.as_u16()
        ));
    }
    let catalog: ModelCatalogResponse = response
        .json()
        .await
        .map_err(|e| format!("catálogo de modelos STT inválido: {e}"))?;
    Ok(catalog.data.into_iter().map(|entry| entry.id).collect())
}

/// Automatically selects the OpenRouter audio endpoint. Dedicated STT models
/// are recognized locally or discovered from OpenRouter's official STT catalog
/// so custom OpenRouter IDs route to the right endpoint automatically.
pub async fn detect_audio_route(model: &str) -> OpenRouterAudioRoute {
    if model_looks_dedicated_stt(model) {
        return OpenRouterAudioRoute::SpeechToText;
    }
    match STT_MODEL_IDS.get_or_try_init(fetch_stt_model_ids).await {
        Ok(ids) if ids.contains(model) => OpenRouterAudioRoute::SpeechToText,
        Ok(_) => OpenRouterAudioRoute::MultimodalLlm,
        Err(error) => {
            log::warn!(
                "OpenRouter: não foi possível detectar automaticamente o endpoint de {}: {}. Usando multimodal.",
                model,
                error
            );
            OpenRouterAudioRoute::MultimodalLlm
        }
    }
}

pub async fn generate_with_audio(
    audio: &[u8],
    ext: &str,
    system_instruction: &str,
    user_prompt: &str,
    model: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<OpenRouterGenerateResult, String> {
    if audio.is_empty() {
        return Err("OpenRouter: o áudio está vazio.".into());
    }
    if api_key.trim().is_empty() {
        return Err("Configure uma chave do OpenRouter em Ajustes → Transcrição.".into());
    }
    let format = audio_format(ext)?;
    let encode_started = Instant::now();
    let encoded = general_purpose::STANDARD.encode(audio);
    let base64_ms = encode_started.elapsed().as_millis() as u64;
    let body = ChatRequest {
        model,
        messages: vec![
            Message {
                role: "system",
                content: MessageContent::Text(system_instruction),
            },
            Message {
                role: "user",
                content: MessageContent::Parts(vec![
                    ContentPart::Text { text: user_prompt },
                    ContentPart::InputAudio {
                        input_audio: InputAudio {
                            data: &encoded,
                            format,
                        },
                    },
                ]),
            },
        ],
        stream: false,
    };

    let started = Instant::now();
    let exchange = async {
        let response = http_client()?
            .post(CHAT_COMPLETIONS_URL)
            .bearer_auth(api_key.trim())
            .headers(app_attribution_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("OpenRouter: falha de rede: {e}"))?;
        let ttfb_ms = started.elapsed().as_millis() as u64;
        let status = response.status();
        let generation_id = response
            .headers()
            .get("x-generation-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let response_body = response
            .text()
            .await
            .map_err(|e| format!("OpenRouter: falha ao ler a resposta: {e}"))?;
        Ok::<_, String>((status, generation_id, response_body, ttfb_ms))
    };
    let (status, generation_id, response_body, ttfb_ms) = tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| format!("OpenRouter: timeout após {} segundos.", timeout.as_secs()))??;
    let request_ms = started.elapsed().as_millis() as u64;
    if !status.is_success() {
        return Err(format!(
            "OpenRouter retornou status {}: {}",
            status.as_u16(),
            truncate_error(&response_body, 800)
        ));
    }
    let parsed: ChatResponse = serde_json::from_str(&response_body)
        .map_err(|e| format!("OpenRouter: resposta inválida: {e}"))?;
    let usage = parsed.usage.unwrap_or_default();
    let text = parsed
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content.trim().to_string())
        .unwrap_or_default();
    Ok(OpenRouterGenerateResult {
        text,
        base64_ms,
        request_ms,
        ttfb_ms: Some(ttfb_ms),
        generation_id,
        reported_total_tokens: usage.total_tokens,
        reported_input_tokens: usage.input_tokens,
        reported_output_tokens: usage.output_tokens,
        reported_audio_seconds: usage.seconds,
        reported_cost_usd: usage.cost,
        transport: AudioTransport::InlineBase64,
        bytes_sent: encoded.len() as u64,
    })
}

/// Text-only OpenRouter call used by Voice Command and opt-in Voice Profile.
pub async fn generate_text(
    system_instruction: &str,
    user_prompt: &str,
    model: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<OpenRouterTextResult, String> {
    if api_key.trim().is_empty() {
        return Err("Configure uma chave do OpenRouter em Ajustes → Transcrição.".into());
    }
    let body = ChatRequest {
        model,
        messages: vec![
            Message {
                role: "system",
                content: MessageContent::Text(system_instruction),
            },
            Message {
                role: "user",
                content: MessageContent::Text(user_prompt),
            },
        ],
        stream: false,
    };
    let started = Instant::now();
    let exchange = async {
        let response = http_client()?
            .post(CHAT_COMPLETIONS_URL)
            .bearer_auth(api_key.trim())
            .headers(app_attribution_headers())
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("OpenRouter: falha de rede: {error}"))?;
        let ttfb_ms = started.elapsed().as_millis() as u64;
        let status = response.status();
        let generation_id = response
            .headers()
            .get("x-generation-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let response_body = response
            .text()
            .await
            .map_err(|error| format!("OpenRouter: falha ao ler a resposta: {error}"))?;
        Ok::<_, String>((status, generation_id, response_body, ttfb_ms))
    };
    let (status, generation_id, response_body, ttfb_ms) = tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| format!("OpenRouter: timeout após {} segundos.", timeout.as_secs()))??;
    let request_ms = started.elapsed().as_millis() as u64;
    if !status.is_success() {
        return Err(format!(
            "OpenRouter retornou status {}: {}",
            status.as_u16(),
            truncate_error(&response_body, 800)
        ));
    }
    let parsed: ChatResponse = serde_json::from_str(&response_body)
        .map_err(|error| format!("OpenRouter: resposta inválida: {error}"))?;
    let usage = parsed.usage.unwrap_or_default();
    let text = parsed
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content.trim().to_string())
        .unwrap_or_default();
    if text.is_empty() {
        return Err("OpenRouter não retornou texto.".into());
    }
    let bytes_sent = serde_json::to_vec(&body)
        .map(|payload| payload.len() as u64)
        .unwrap_or_default();
    Ok(OpenRouterTextResult {
        text,
        request_ms,
        ttfb_ms: Some(ttfb_ms),
        generation_id,
        reported_total_tokens: usage.total_tokens,
        reported_input_tokens: usage.input_tokens,
        reported_output_tokens: usage.output_tokens,
        reported_cost_usd: usage.cost,
        bytes_sent,
    })
}

/// Concise AssemblyAI options forwarded via OpenRouter provider passthrough.
/// Intentionally omits `conversation_context` (unrelated past dictations) to
/// avoid bias and hallucinations.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AssemblyAiOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keyterms_prompt: Vec<String>,
}

pub async fn transcribe_audio(
    audio: &[u8],
    ext: &str,
    model: &str,
    api_key: &str,
    timeout: Duration,
    vocabulary_prompt: Option<&str>,
) -> Result<OpenRouterGenerateResult, String> {
    transcribe_audio_with_assembly(
        audio,
        ext,
        model,
        api_key,
        timeout,
        vocabulary_prompt,
        None,
        &[],
    )
    .await
}

/// Sends one dedicated-STT request. `keyterms` are only forwarded to
/// ElevenLabs models; if OpenRouter refuses them (400/422) the request is
/// retried once without them so vocabulary never costs a dictation.
#[allow(clippy::too_many_arguments)]
pub async fn transcribe_audio_with_assembly(
    audio: &[u8],
    ext: &str,
    model: &str,
    api_key: &str,
    timeout: Duration,
    vocabulary_prompt: Option<&str>,
    assembly: Option<&AssemblyAiOptions>,
    keyterms: &[String],
) -> Result<OpenRouterGenerateResult, String> {
    if audio.is_empty() {
        return Err("OpenRouter STT: o áudio está vazio.".into());
    }
    if api_key.trim().is_empty() {
        return Err("Configure uma chave do OpenRouter em Ajustes → Transcrição.".into());
    }
    let capabilities =
        crate::pipeline_run::transport_capabilities("openrouter", "audio/transcriptions");
    if capabilities
        .best_supported(&[AudioTransport::Multipart], audio.len() as u64)
        .is_none()
    {
        return Err("OpenRouter STT: áudio excede o limite multipart de 25 MB.".into());
    }
    let keyterms: &[String] = if is_elevenlabs_model(model) {
        keyterms
    } else {
        &[]
    };
    let form = build_transcription_form(audio, ext, model, vocabulary_prompt, assembly, keyterms)?;
    match send_transcription(form, api_key, timeout, audio.len()).await {
        Err(SttFailure::Rejected(message)) if !keyterms.is_empty() => {
            log::warn!(
                "OpenRouter STT: {} keyterms recusados para {model}; repetindo sem eles: {message}",
                keyterms.len()
            );
            let form =
                build_transcription_form(audio, ext, model, vocabulary_prompt, assembly, &[])?;
            send_transcription(form, api_key, timeout, audio.len())
                .await
                .map_err(SttFailure::into_message)
        }
        result => result.map_err(SttFailure::into_message),
    }
}

fn build_transcription_form(
    audio: &[u8],
    ext: &str,
    model: &str,
    vocabulary_prompt: Option<&str>,
    assembly: Option<&AssemblyAiOptions>,
    keyterms: &[String],
) -> Result<multipart::Form, String> {
    let format = audio_format(ext)?;
    let filename = format!("audio.{format}");
    let mime = match format {
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "webm" => "audio/webm",
        "aac" => "audio/aac",
        _ => "application/octet-stream",
    };
    let part = multipart::Part::bytes(audio.to_vec())
        .file_name(filename)
        .mime_str(mime)
        .map_err(|error| format!("OpenRouter STT: MIME inválido: {error}"))?;
    let is_assembly = is_assemblyai_model(model);
    let is_whisper = model.to_ascii_lowercase().contains("whisper");
    let mut form = multipart::Form::new()
        .text("model", model.to_string())
        .part("file", part);
    // Omitting `language` on AssemblyAI and modern multilingual STT models
    // preserves native pt-BR + English code-switching. Keep `language=pt` only
    // on Whisper routes where it anchors pt-BR phonetics.
    if is_whisper {
        form = form.text("language", "pt");
    }
    if is_assembly {
        if let Some(options) = assembly {
            if options.prompt.is_some() || !options.keyterms_prompt.is_empty() {
                let provider = serde_json::json!({ "options": { "assemblyai": options } });
                form = form.text("provider", provider.to_string());
            }
        }
    } else if is_elevenlabs_model(model) {
        form = form.text("provider", elevenlabs_provider_options().to_string());
        for term in keyterms {
            form = form.text("keyterms[]", term.clone());
        }
    } else if let Some(provider) = whisper_provider_preferences(model, vocabulary_prompt) {
        form = form.text(
            "provider",
            serde_json::to_string(&provider).map_err(|error| error.to_string())?,
        );
    }
    Ok(form)
}

enum SttFailure {
    /// 400/422: OpenRouter refused the request shape.
    Rejected(String),
    Other(String),
}

impl SttFailure {
    fn into_message(self) -> String {
        match self {
            Self::Rejected(message) | Self::Other(message) => message,
        }
    }
}

async fn send_transcription(
    form: multipart::Form,
    api_key: &str,
    timeout: Duration,
    audio_len: usize,
) -> Result<OpenRouterGenerateResult, SttFailure> {
    let started = Instant::now();
    let exchange = async {
        let response = http_client()?
            .post(TRANSCRIPTIONS_URL)
            .bearer_auth(api_key.trim())
            .headers(app_attribution_headers())
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("OpenRouter STT: falha de rede: {e}"))?;
        let ttfb_ms = started.elapsed().as_millis() as u64;
        let status = response.status();
        let generation_id = response
            .headers()
            .get("x-generation-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let response_body = response
            .text()
            .await
            .map_err(|e| format!("OpenRouter STT: falha ao ler a resposta: {e}"))?;
        Ok::<_, String>((status, generation_id, response_body, ttfb_ms))
    };
    let (status, generation_id, response_body, ttfb_ms) = tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| {
            SttFailure::Other(format!(
                "OpenRouter STT: timeout após {} segundos.",
                timeout.as_secs()
            ))
        })?
        .map_err(SttFailure::Other)?;
    let request_ms = started.elapsed().as_millis() as u64;
    if !status.is_success() {
        let message = format!(
            "OpenRouter STT retornou status {}: {}",
            status.as_u16(),
            truncate_error(&response_body, 800)
        );
        return Err(if matches!(status.as_u16(), 400 | 422) {
            SttFailure::Rejected(message)
        } else {
            SttFailure::Other(message)
        });
    }

    let parsed: TranscriptionResponse = serde_json::from_str(&response_body)
        .map_err(|e| SttFailure::Other(format!("OpenRouter STT: resposta inválida: {e}")))?;
    let text = parsed.text.trim().to_string();
    let usage = parsed.usage.unwrap_or_default();
    Ok(OpenRouterGenerateResult {
        text,
        base64_ms: 0,
        request_ms,
        ttfb_ms: Some(ttfb_ms),
        generation_id,
        reported_total_tokens: usage.total_tokens,
        reported_input_tokens: usage.input_tokens,
        reported_output_tokens: usage.output_tokens,
        reported_audio_seconds: usage.seconds,
        reported_cost_usd: usage.cost,
        transport: AudioTransport::Multipart,
        bytes_sent: audio_len as u64,
    })
}

fn audio_format(ext: &str) -> Result<&'static str, String> {
    match ext
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "wav" | "wave" => Ok("wav"),
        "mp3" => Ok("mp3"),
        "flac" => Ok("flac"),
        "m4a" => Ok("m4a"),
        "ogg" | "oga" => Ok("ogg"),
        "webm" => Ok("webm"),
        "aac" => Ok("aac"),
        other => Err(format!(
            "OpenRouter: formato de áudio não suportado: {other}"
        )),
    }
}

fn truncate_error(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let shortened: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_matches_openrouter_audio_contract() {
        let body = ChatRequest {
            model: "openai/gpt-audio-mini",
            messages: vec![
                Message {
                    role: "system",
                    content: MessageContent::Text("política"),
                },
                Message {
                    role: "user",
                    content: MessageContent::Parts(vec![
                        ContentPart::Text { text: "transcreva" },
                        ContentPart::InputAudio {
                            input_audio: InputAudio {
                                data: "UklGRg==",
                                format: "wav",
                            },
                        },
                    ]),
                },
            ],
            stream: false,
        };
        let json = serde_json::to_value(body).unwrap();
        assert_eq!(json["model"], "openai/gpt-audio-mini");
        assert_eq!(json["messages"][0]["role"], "system");
        assert_eq!(json["messages"][0]["content"], "política");
        assert_eq!(json["messages"][1]["content"][1]["type"], "input_audio");
        assert_eq!(
            json["messages"][1]["content"][1]["input_audio"]["format"],
            "wav"
        );
    }

    #[test]
    fn request_matches_openrouter_transcription_contract() {
        let capabilities =
            crate::pipeline_run::transport_capabilities("openrouter", "audio/transcriptions");
        assert!(capabilities.multipart);
        assert!(!capabilities.inline_base64);
        assert_eq!(capabilities.max_multipart_bytes, Some(25 * 1024 * 1024));
    }

    #[test]
    fn attributes_requests_to_sonora() {
        let headers = app_attribution_headers();
        assert_eq!(headers.get(REFERER).unwrap(), APP_REFERER);
        assert_eq!(headers.get("X-OpenRouter-Title").unwrap(), APP_TITLE);
    }

    #[test]
    fn automatic_route_recognizes_common_stt_and_multimodal_ids() {
        assert!(model_looks_dedicated_stt("assemblyai/universal-3-6-pro"));
        assert!(model_looks_dedicated_stt("openai/whisper-large-v3"));
        assert!(model_looks_dedicated_stt("openai/gpt-4o-mini-transcribe"));
        assert!(model_looks_dedicated_stt("openai/gpt-4o-transcribe"));
        assert!(model_looks_dedicated_stt("elevenlabs/scribe-v2"));
        assert!(!model_looks_dedicated_stt("openai/gpt-audio-mini"));
    }

    #[test]
    fn assembly_options_omit_empty_fields_and_have_no_conversation_context() {
        let opts = AssemblyAiOptions {
            prompt: Some("Voice dictation in Brazilian Portuguese and English.".into()),
            keyterms_prompt: vec!["Sonora".into()],
        };
        let json = serde_json::to_value(&opts).unwrap();
        assert_eq!(
            json["prompt"],
            "Voice dictation in Brazilian Portuguese and English."
        );
        assert_eq!(json["keyterms_prompt"], serde_json::json!(["Sonora"]));
        assert!(json.get("conversation_context").is_none());
    }

    #[test]
    fn elevenlabs_options_disable_tags_and_fillers() {
        let value = elevenlabs_provider_options();
        let options = &value["options"]["elevenlabs"];
        assert_eq!(options["tag_audio_events"], false);
        assert_eq!(options["no_verbatim"], true);
        assert!(is_elevenlabs_model(" ElevenLabs/scribe-v2 "));
        assert!(!is_elevenlabs_model("assemblyai/universal-3-6-pro"));
    }

    #[test]
    fn stt_catalog_shape_extracts_model_ids() {
        let catalog: ModelCatalogResponse = serde_json::from_str(
            r#"{"data":[{"id":"assemblyai/universal-3-6-pro"},{"id":"openai/whisper-large-v3"}]}"#,
        )
        .unwrap();
        let ids: HashSet<_> = catalog.data.into_iter().map(|entry| entry.id).collect();
        assert!(ids.contains("assemblyai/universal-3-6-pro"));
        assert!(ids.contains("openai/whisper-large-v3"));
    }

    #[test]
    fn whisper_transcription_is_pinned_to_groq_without_fallback() {
        let value = serde_json::to_value(
            whisper_provider_preferences("openai/whisper-large-v3", None).unwrap(),
        )
        .expect("serialize provider preferences");
        assert_eq!(value["only"][0], "groq");
        assert_eq!(value["allow_fallbacks"], false);
        assert!(value.get("options").is_none());
    }

    #[test]
    fn whisper_vocabulary_prompt_is_forwarded_to_groq_only() {
        let value = serde_json::to_value(
            whisper_provider_preferences("openai/whisper-large-v3-turbo", Some("Sonora, Tauri"))
                .unwrap(),
        )
        .expect("serialize provider preferences");
        assert_eq!(value["options"]["groq"]["prompt"], "Sonora, Tauri");
    }
}
