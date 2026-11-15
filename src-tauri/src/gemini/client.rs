//! Shared HTTP client, model ids, timeouts and response parsing.

use serde::{Deserialize, Serialize};
use std::{sync::OnceLock, time::Duration};

/// Multimodal model used for refinement and pronunciation.
/// Higher-capability model used for pronunciation evaluation (CEFR rubric).
pub const PRONUNCIATION_MODEL: &str = "gemini-3.5-flash";

pub const API_ROOT: &str = "https://generativelanguage.googleapis.com/v1beta";
pub const UPLOAD_ROOT: &str = "https://generativelanguage.googleapis.com/upload/v1beta";

/// Default client idle timeout (must cover the longest single request we make).
const CLIENT_TIMEOUT: Duration = Duration::from_secs(180);

pub const TIMEOUT_UPLOAD: Duration = Duration::from_secs(60);
pub const TIMEOUT_POLL: Duration = Duration::from_secs(15);
pub const TIMEOUT_GENERATE_MIN: Duration = Duration::from_secs(10);
pub const TIMEOUT_GENERATE_MAX: Duration = Duration::from_secs(20);
pub const TIMEOUT_DELETE: Duration = Duration::from_secs(15);
/// Max wall time waiting for Files API PROCESSING → ACTIVE.
pub const TIMEOUT_FILE_READY: Duration = Duration::from_secs(90);
pub const POLL_INTERVAL: Duration = Duration::from_millis(800);

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub fn http_client() -> Result<&'static reqwest::Client, String> {
    if let Some(client) = HTTP_CLIENT.get() {
        return Ok(client);
    }
    let client = reqwest::Client::builder()
        .timeout(CLIENT_TIMEOUT)
        .build()
        .map_err(|e| format!("falha ao construir o cliente http do Gemini: {}", e))?;
    let _ = HTTP_CLIENT.set(client);
    HTTP_CLIENT
        .get()
        .ok_or_else(|| "falha ao inicializar o cliente http do Gemini".to_string())
}

/// Maps a file extension to a Gemini-accepted audio MIME type.
pub fn mime_for_ext(ext: &str) -> &'static str {
    let e: String = ext
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    match e.as_str() {
        "mp3" => "audio/mp3",
        "m4a" | "mp4" | "aac" => "audio/aac",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "aiff" | "aif" => "audio/aiff",
        "webm" => "audio/webm",
        "wav" | "wave" => "audio/wav",
        _ => "audio/wav",
    }
}

pub fn require_api_key(api_key: &str) -> Result<(), String> {
    if api_key.trim().is_empty() {
        return Err("a chave de API do Google não está configurada".to_string());
    }
    Ok(())
}

/// Adaptive wall timeout for a complete Gemini `generateContent` response.
///
/// Known duration adds one second per complete 30 seconds of audio. When the
/// duration is unavailable (for example, some compressed uploads), payload
/// size adds one second per complete MiB. Both paths are clamped to 10–20s.
pub fn adaptive_generate_timeout(duration_ms: Option<u64>, audio_bytes: usize) -> Duration {
    let scale = match duration_ms {
        Some(ms) => ms / 30_000,
        None => audio_bytes as u64 / 1_048_576,
    };
    Duration::from_secs(
        (TIMEOUT_GENERATE_MIN.as_secs() + scale).min(TIMEOUT_GENERATE_MAX.as_secs()),
    )
}

pub fn is_transcribe_model(model: &str) -> bool {
    let lower = model.to_lowercase();
    lower.contains("transcribe")
}

/* --------------------------- generateContent shapes --------------------------- */

#[derive(Debug, Clone, Serialize)]
pub struct GenerateContentRequest {
    /// Stable behavioral policy, kept separate from request-specific evidence.
    #[serde(skip_serializing_if = "Option::is_none", rename = "systemInstruction")]
    pub system_instruction: Option<Content>,
    pub contents: Vec<Content>,
    /// Prefer deterministic transcription; supported on Generative Language API.
    #[serde(skip_serializing_if = "Option::is_none", rename = "generationConfig")]
    pub generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenerationConfig {
    /// Sampling parameters are omitted for Gemini 3.5+ compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "thinkingConfig")]
    pub thinking_config: Option<ThinkingConfig>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThinkingConfig {
    #[serde(rename = "thinkingLevel")]
    pub thinking_level: &'static str,
}

impl GenerateContentRequest {
    pub fn with_parts(parts: Vec<Part>) -> Self {
        Self {
            system_instruction: None,
            contents: vec![Content { parts }],
            generation_config: Some(GenerationConfig {
                temperature: None,
                thinking_config: Some(ThinkingConfig {
                    thinking_level: "minimal",
                }),
            }),
        }
    }

    pub fn with_system_instruction(mut self, instruction: &str) -> Self {
        if !instruction.trim().is_empty() {
            self.system_instruction = Some(Content {
                parts: vec![Part::Text {
                    text: instruction.to_string(),
                }],
            });
        }
        self
    }

    /// Direct STT is a simple task: use the fastest supported reasoning level
    /// and leave temperature at the Gemini 3 default.
    pub fn for_fast_accurate(mut self) -> Self {
        self.generation_config = Some(GenerationConfig {
            temperature: None,
            thinking_config: Some(ThinkingConfig {
                thinking_level: "minimal",
            }),
        });
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Content {
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Part {
    Text {
        text: String,
    },
    Inline {
        #[serde(rename = "inline_data")]
        inline_data: InlineData,
    },
    File {
        #[serde(rename = "file_data")]
        file_data: FileData,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct InlineData {
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileData {
    pub mime_type: String,
    pub file_uri: String,
}

/* ---------------------------- Interactions API shapes ---------------------------- */

#[derive(Debug, Clone, Serialize)]
pub struct InteractionRequest {
    pub model: String,
    pub input: Vec<InteractionInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<InteractionGenerationConfig>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InteractionInput {
    Text {
        text: String,
    },
    Audio {
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
        mime_type: String,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InteractionGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcription_config: Option<InteractionTranscriptionConfig>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InteractionTranscriptionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_codes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_vocabulary: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<InteractionTranscriptionMode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractionTranscriptionMode {
    #[serde(rename = "type")]
    pub mode_type: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct InteractionResponse {
    #[serde(default)]
    #[allow(dead_code)]
    pub id: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub status: Option<String>,
    #[serde(default)]
    pub steps: Vec<InteractionStep>,
    #[serde(default)]
    pub usage: Option<InteractionUsage>,
    #[serde(default)]
    pub error: Option<ApiErrorBody>,
}

#[derive(Debug, Deserialize)]
pub struct InteractionStep {
    #[serde(default, rename = "type")]
    pub step_type: String,
    #[serde(default)]
    pub content: Option<Vec<InteractionContentPart>>,
}

#[derive(Debug, Deserialize)]
pub struct InteractionContentPart {
    #[serde(default, rename = "type")]
    #[allow(dead_code)]
    pub part_type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InteractionUsage {
    #[serde(default)]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub total_input_tokens: Option<u64>,
    #[serde(default)]
    pub total_output_tokens: Option<u64>,
    #[serde(default)]
    pub total_thought_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateContentResponse {
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(default)]
    pub error: Option<ApiErrorBody>,
    #[serde(default, rename = "usageMetadata")]
    pub usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Default, Deserialize)]
pub struct GeminiUsageMetadata {
    #[serde(default, rename = "promptTokenCount")]
    pub prompt_token_count: Option<u64>,
    #[serde(default, rename = "candidatesTokenCount")]
    pub candidates_token_count: Option<u64>,
    #[serde(default, rename = "totalTokenCount")]
    pub total_token_count: Option<u64>,
    #[serde(default, rename = "thoughtsTokenCount")]
    pub thoughts_token_count: Option<u64>,
}

#[derive(Debug)]
pub struct GenerateContentOutcome {
    pub text: String,
    pub duration_ms: u64,
    pub usage: crate::pipeline_run::UsageRecord,
}

#[derive(Debug, Deserialize)]
pub struct Candidate {
    #[serde(default)]
    pub content: Option<ResponseContent>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseContent {
    #[serde(default)]
    pub parts: Vec<ResponsePart>,
}

#[derive(Debug, Deserialize)]
pub struct ResponsePart {
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApiErrorBody {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub code: Option<i64>,
    #[serde(default)]
    #[allow(dead_code)]
    pub status: Option<String>,
}

/// Extracts concatenated text from a successful generateContent JSON body.
pub fn extract_text(parsed: GenerateContentResponse) -> Result<String, String> {
    if let Some(err) = parsed.error {
        let msg = err.message.unwrap_or_else(|| "erro desconhecido".into());
        return Err(format!(
            "Gemini API error{}: {}",
            err.code.map(|c| format!(" ({c})")).unwrap_or_default(),
            msg
        ));
    }

    let text = parsed
        .candidates
        .into_iter()
        .next()
        .and_then(|c| c.content)
        .map(|c| {
            c.parts
                .into_iter()
                .filter_map(|p| p.text)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err("o Gemini não retornou texto".to_string());
    }
    Ok(trimmed)
}

pub fn generate_url(model: &str, _api_key: &str) -> String {
    format!("{}/models/{}:generateContent", API_ROOT, model)
}

/// POST generateContent with a per-request timeout override.
/// Returns `(text, generate_wall_ms)`.
pub async fn generate_content_with_model(
    api_key: &str,
    model: &str,
    body: &GenerateContentRequest,
    timeout: Duration,
) -> Result<GenerateContentOutcome, String> {
    require_api_key(api_key)?;
    let client = http_client()?;
    let url = generate_url(model, api_key);
    let t0 = std::time::Instant::now();

    // If a dedicated transcribe model is called via generateContent, strip unsupported fields.
    let mut sanitized_body;
    let effective_body = if is_transcribe_model(model) {
        sanitized_body = body.clone();
        sanitized_body.system_instruction = None;
        if let Some(ref mut gc) = sanitized_body.generation_config {
            gc.thinking_config = None;
        }
        &sanitized_body
    } else {
        body
    };

    let request = async {
        let response = client
            .post(url)
            .header("x-goog-api-key", api_key)
            .json(effective_body)
            .send()
            .await
            .map_err(|e| format!("falha na requisição ao Gemini: {}", e.without_url()))?;
        let status = response.status().as_u16();
        let body_text = response
            .text()
            .await
            .map_err(|e| format!("falha ao ler a resposta do Gemini: {}", e.without_url()))?;
        Ok::<_, String>((status, body_text))
    };

    let (status, body_text) = tokio::time::timeout(timeout, request).await.map_err(|_| {
        format!(
            "timeout ao aguardar resposta completa do Gemini ({}s)",
            timeout.as_secs()
        )
    })??;
    let generate_ms = t0.elapsed().as_millis() as u64;
    if status != 200 {
        return Err(format!("Gemini retornou HTTP {status}. Confira as credenciais, os limites e a disponibilidade do modelo."));
    }

    let parsed: GenerateContentResponse = serde_json::from_str(&body_text)
        .map_err(|e| format!("falha ao interpretar a resposta do Gemini: {}", e))?;
    let usage = parsed
        .usage_metadata
        .as_ref()
        .map(|metadata| {
            let mut usage = crate::pipeline_run::UsageRecord {
                input_tokens: metadata.prompt_token_count,
                output_tokens: metadata.candidates_token_count,
                total_tokens: metadata.total_token_count,
                ..Default::default()
            };
            if let Some(thoughts) = metadata.thoughts_token_count {
                usage
                    .metadata
                    .insert("thoughts_tokens".into(), thoughts.into());
            }
            usage
        })
        .unwrap_or_default();
    let text = extract_text(parsed)?;
    Ok(GenerateContentOutcome {
        text,
        duration_ms: generate_ms,
        usage,
    })
}

pub fn interactions_url(_api_key: &str) -> String {
    format!("{}/interactions", API_ROOT)
}

/// POST to the Gemini Interactions API (/v1beta/interactions).
/// This is the primary, first-class surface for `gemini-3.5-transcribe`.
pub async fn interact_with_model(
    api_key: &str,
    body: &InteractionRequest,
    timeout: Duration,
) -> Result<GenerateContentOutcome, String> {
    require_api_key(api_key)?;
    let client = http_client()?;
    let url = interactions_url(api_key);
    let t0 = std::time::Instant::now();

    let request = async {
        let response = client
            .post(url)
            .header("x-goog-api-key", api_key)
            .json(body)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "falha na requisição ao Gemini (interactions): {}",
                    e.without_url()
                )
            })?;
        let status = response.status().as_u16();
        let body_text = response.text().await.map_err(|e| {
            format!(
                "falha ao ler a resposta do Gemini (interactions): {}",
                e.without_url()
            )
        })?;
        Ok::<_, String>((status, body_text))
    };

    let (status, body_text) = tokio::time::timeout(timeout, request).await.map_err(|_| {
        format!(
            "timeout ao aguardar resposta do Gemini (interactions) ({}s)",
            timeout.as_secs()
        )
    })??;
    let generate_ms = t0.elapsed().as_millis() as u64;
    if status != 200 {
        return Err(format!("Gemini retornou HTTP {status}. Confira as credenciais, os limites e a disponibilidade do modelo."));
    }

    let parsed: InteractionResponse = serde_json::from_str(&body_text).map_err(|e| {
        format!(
            "falha ao interpretar resposta do Gemini (interactions): {}",
            e
        )
    })?;

    if let Some(err) = parsed.error {
        let msg = err.message.unwrap_or_else(|| "erro desconhecido".into());
        return Err(format!(
            "Gemini API error{}: {}",
            err.code.map(|c| format!(" ({c})")).unwrap_or_default(),
            msg
        ));
    }

    let text = parsed
        .steps
        .into_iter()
        .filter(|s| s.step_type == "model_output")
        .filter_map(|s| s.content)
        .flat_map(|parts| parts.into_iter().filter_map(|p| p.text))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    let usage = parsed
        .usage
        .as_ref()
        .map(|u| {
            let mut rec = crate::pipeline_run::UsageRecord {
                input_tokens: u.total_input_tokens,
                output_tokens: u.total_output_tokens,
                total_tokens: u.total_tokens,
                ..Default::default()
            };
            if let Some(thoughts) = u.total_thought_tokens {
                rec.metadata
                    .insert("thoughts_tokens".into(), thoughts.into());
            }
            rec
        })
        .unwrap_or_default();

    Ok(GenerateContentOutcome {
        text,
        duration_ms: generate_ms,
        usage,
    })
}

/// Build generateContent body with a real system instruction + user text + inline audio.
pub fn build_inline_request(
    system_instruction: &str,
    user_prompt: &str,
    mime: &str,
    base64_data: &str,
) -> GenerateContentRequest {
    GenerateContentRequest::with_parts(vec![
        Part::Text {
            text: user_prompt.to_string(),
        },
        Part::Inline {
            inline_data: InlineData {
                mime_type: mime.to_string(),
                data: base64_data.to_string(),
            },
        },
    ])
    .with_system_instruction(system_instruction)
}

/// Build generateContent body with a real system instruction + user text + remote file.
pub fn build_file_request(
    system_instruction: &str,
    user_prompt: &str,
    mime: &str,
    file_uri: &str,
) -> GenerateContentRequest {
    GenerateContentRequest::with_parts(vec![
        Part::Text {
            text: user_prompt.to_string(),
        },
        Part::File {
            file_data: FileData {
                mime_type: mime.to_string(),
                file_uri: file_uri.to_string(),
            },
        },
    ])
    .with_system_instruction(system_instruction)
}

/// Build an Interactions API request with inline audio.
pub fn build_interaction_inline_request(
    model: &str,
    mime: &str,
    base64_data: &str,
    text_prompt: Option<&str>,
    custom_vocabulary: Option<Vec<String>>,
) -> InteractionRequest {
    let mut input = Vec::new();
    if let Some(text) = text_prompt {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            input.push(InteractionInput::Text {
                text: trimmed.to_string(),
            });
        }
    }
    input.push(InteractionInput::Audio {
        data: Some(base64_data.to_string()),
        uri: None,
        mime_type: mime.to_string(),
    });

    InteractionRequest {
        model: model.to_string(),
        input,
        generation_config: Some(InteractionGenerationConfig {
            transcription_config: Some(InteractionTranscriptionConfig {
                // None enables native automatic language identification and code-switching
                language_codes: None,
                custom_vocabulary,
                mode: Some(InteractionTranscriptionMode { mode_type: "smart" }),
            }),
        }),
    }
}

/// Build an Interactions API request with Files API remote audio.
pub fn build_interaction_file_request(
    model: &str,
    mime: &str,
    file_uri: &str,
    text_prompt: Option<&str>,
    custom_vocabulary: Option<Vec<String>>,
) -> InteractionRequest {
    let mut input = Vec::new();
    if let Some(text) = text_prompt {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            input.push(InteractionInput::Text {
                text: trimmed.to_string(),
            });
        }
    }
    input.push(InteractionInput::Audio {
        data: None,
        uri: Some(file_uri.to_string()),
        mime_type: mime.to_string(),
    });

    InteractionRequest {
        model: model.to_string(),
        input,
        generation_config: Some(InteractionGenerationConfig {
            transcription_config: Some(InteractionTranscriptionConfig {
                // None enables native automatic language identification and code-switching
                language_codes: None,
                custom_vocabulary,
                mode: Some(InteractionTranscriptionMode { mode_type: "smart" }),
            }),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_mapping() {
        assert_eq!(mime_for_ext("wav"), "audio/wav");
        assert_eq!(mime_for_ext("MP3"), "audio/mp3");
        assert_eq!(mime_for_ext("m4a"), "audio/aac");
        assert_eq!(mime_for_ext("flac"), "audio/flac");
        assert_eq!(mime_for_ext("unknown"), "audio/wav");
    }

    #[test]
    fn extract_text_from_candidates() {
        let json = r#"{
            "candidates": [{
                "content": { "parts": [{ "text": "  olá mundo  " }] }
            }]
        }"#;
        let parsed: GenerateContentResponse = serde_json::from_str(json).unwrap();
        assert_eq!(extract_text(parsed).unwrap(), "olá mundo");
    }

    #[test]
    fn extract_text_api_error() {
        let json = r#"{ "error": { "code": 403, "message": "permission denied" } }"#;
        let parsed: GenerateContentResponse = serde_json::from_str(json).unwrap();
        let err = extract_text(parsed).unwrap_err();
        assert!(err.contains("403"));
        assert!(err.contains("permission denied"));
    }

    #[test]
    fn empty_key_rejected() {
        assert!(require_api_key("").is_err());
        assert!(require_api_key("   ").is_err());
    }

    #[test]
    fn adaptive_generate_timeout_is_clamped_and_scales_with_audio() {
        assert_eq!(adaptive_generate_timeout(Some(0), 0), TIMEOUT_GENERATE_MIN);
        assert_eq!(adaptive_generate_timeout(Some(1), 0), TIMEOUT_GENERATE_MIN);
        assert_eq!(
            adaptive_generate_timeout(Some(120_000), 0),
            Duration::from_secs(14)
        );
        assert_eq!(
            adaptive_generate_timeout(Some(900_000), 0),
            TIMEOUT_GENERATE_MAX
        );
        assert_eq!(
            adaptive_generate_timeout(None, 5 * 1_048_576),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn fast_accurate_uses_explicit_minimal_thinking_without_temperature() {
        let request =
            build_inline_request("política", "transcreva", "audio/wav", "AA==").for_fast_accurate();
        let json = serde_json::to_value(request).unwrap();

        assert_eq!(
            json.pointer("/systemInstruction/parts/0/text"),
            Some(&serde_json::Value::String("política".into()))
        );
        assert_eq!(
            json.pointer("/contents/0/parts/0/text"),
            Some(&serde_json::Value::String("transcreva".into()))
        );

        assert_eq!(
            json.pointer("/generationConfig/thinkingConfig/thinkingLevel"),
            Some(&serde_json::Value::String("minimal".into()))
        );
        assert!(json.pointer("/generationConfig/temperature").is_none());
    }

    #[test]
    fn detects_transcribe_model_properly() {
        assert!(is_transcribe_model("gemini-3.5-transcribe"));
        assert!(is_transcribe_model("models/gemini-3.5-transcribe"));
        assert!(is_transcribe_model("google/gemini-3.5-transcribe"));
        assert!(!is_transcribe_model("gemini-3.5-flash-lite"));
        assert!(!is_transcribe_model("gemini-3.6-flash"));
    }

    #[test]
    fn interaction_request_serializes_with_transcription_config() {
        let req = build_interaction_inline_request(
            "gemini-3.5-transcribe",
            "audio/wav",
            "AQID",
            Some("estilo confiável"),
            Some(vec!["Sonora".into(), "Tauri".into()]),
        );
        let val = serde_json::to_value(&req).unwrap();

        assert_eq!(val["model"], "gemini-3.5-transcribe");
        assert_eq!(val["input"][0]["type"], "text");
        assert_eq!(val["input"][0]["text"], "estilo confiável");
        assert_eq!(val["input"][1]["type"], "audio");
        assert_eq!(val["input"][1]["data"], "AQID");
        assert_eq!(val["input"][1]["mime_type"], "audio/wav");

        assert!(val["generation_config"]["transcription_config"]["language_codes"].is_null());
        assert_eq!(
            val["generation_config"]["transcription_config"]["custom_vocabulary"],
            serde_json::json!(["Sonora", "Tauri"])
        );
        assert_eq!(
            val["generation_config"]["transcription_config"]["mode"]["type"],
            "smart"
        );
    }

    #[test]
    fn interaction_response_extracts_model_output_text() {
        let json = r#"{
            "id": "interaction-123",
            "status": "completed",
            "steps": [
                {
                    "type": "thought",
                    "signature": "sig"
                },
                {
                    "type": "model_output",
                    "content": [
                        { "type": "text", "text": "Transcrição de alta fidelidade" }
                    ]
                }
            ],
            "usage": {
                "total_tokens": 30,
                "total_input_tokens": 25,
                "total_output_tokens": 5
            }
        }"#;

        let parsed: InteractionResponse = serde_json::from_str(json).unwrap();
        let text = parsed
            .steps
            .into_iter()
            .filter(|s| s.step_type == "model_output")
            .filter_map(|s| s.content)
            .flat_map(|parts| parts.into_iter().filter_map(|p| p.text))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();

        assert_eq!(text, "Transcrição de alta fidelidade");
    }
    #[test]
    fn credentials_are_never_part_of_request_urls() {
        for url in [
            generate_url("test-model", "synthetic-private-key"),
            interactions_url("synthetic-private-key"),
        ] {
            assert!(!url.contains("synthetic-private-key"));
            assert!(!url.contains("key="));
        }
    }
}
