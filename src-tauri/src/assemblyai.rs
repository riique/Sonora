//! Direct AssemblyAI pre-recorded Speech-to-Text API client (`assemblyai-stt-3-7`).
//!
//! Official pre-recorded flow:
//! 1. `POST /v2/upload` — raw binary upload returning a temporary `upload_url`.
//! 2. `POST /v2/transcript` — submit job with `speech_models: ["assemblyai-stt-3-7"]`,
//!    `language_detection: true`, `punctuate: true`, `format_text: true`, a concise
//!    non-instructional domain `prompt`, and `keyterms_prompt` from custom vocabulary.
//! 3. `GET /v2/transcript/{id}` — poll until `completed` or `error`.
//!
//! Authentication sends the raw API key in the `Authorization` header (no `Bearer`
//! prefix, per AssemblyAI's API specification).

use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::OnceLock,
    time::{Duration, Instant},
};

pub const DEFAULT_BASE_URL: &str = "https://api.assemblyai.com";
pub const MODEL_STT_3_7: &str = "assemblyai-stt-3-7";
pub const MAX_DIRECT_KEYTERMS: usize = 1000;
pub const MAX_OPENROUTER_KEYTERMS: usize = 100;
const MAX_KEYTERM_CHARS: usize = 50;
const MAX_KEYTERM_WORDS: usize = 6;

/// Short, neutral topical description for AssemblyAI's contextual `prompt`.
/// Per AssemblyAI documentation, `prompt` describes the domain/scenario (5–15
/// words) rather than behavioral/formatting rules, preventing hallucinations
/// while grounding pt-BR + English code-switching dictations.
pub const DEFAULT_CONTEXT_PROMPT: &str = "Voice dictation in Brazilian Portuguese and English.";

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
        .ok_or_else(|| "AssemblyAI HTTP client unavailable".into())
}

/// Builds a clean `keyterms_prompt` list from the user's enabled custom
/// vocabulary. Strict terms come first; phonetic mishearing aliases are never
/// sent as keyterms because they represent the *wrong* spellings.
/// Phrases are capped at 6 words and 50 characters per AssemblyAI guidance.
pub fn build_keyterms(
    vocab: &[crate::vocabulary::VocabularyTerm],
    max_terms: usize,
) -> Vec<String> {
    if max_terms == 0 {
        return Vec::new();
    }
    let mut ordered: Vec<&crate::vocabulary::VocabularyTerm> =
        vocab.iter().filter(|term| term.enabled).collect();
    ordered.sort_by_key(|term| !term.strict);

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for term in ordered {
        let canonical = term.canonical.trim();
        if canonical.is_empty()
            || canonical.chars().any(char::is_control)
            || canonical.chars().count() > MAX_KEYTERM_CHARS
            || canonical.split_whitespace().count() > MAX_KEYTERM_WORDS
        {
            continue;
        }
        if seen.insert(canonical.to_lowercase()) {
            out.push(canonical.to_string());
            if out.len() >= max_terms {
                break;
            }
        }
    }
    out
}

/// Returns the concise contextual prompt for AssemblyAI models (both direct
/// `assemblyai-stt-3-7` and OpenRouter `assemblyai/universal-3-6-pro`).
pub fn context_prompt() -> &'static str {
    DEFAULT_CONTEXT_PROMPT
}

pub fn adaptive_upload_timeout(audio_bytes: usize) -> Duration {
    let extra_secs = (audio_bytes as u64) / 32_768;
    Duration::from_secs((60 + extra_secs).clamp(60, 180))
}

pub fn adaptive_poll_timeout(duration_ms: Option<u64>, audio_bytes: usize) -> Duration {
    let estimated_secs = duration_ms
        .map(|ms| ms.div_ceil(1000))
        .unwrap_or_else(|| (audio_bytes as u64) / 32_000);
    let budget = 90_u64.max(estimated_secs.saturating_mul(3).saturating_add(60));
    Duration::from_secs(budget.min(600))
}

#[derive(Debug, Clone)]
pub struct AssemblyAiTranscribeRequest<'a> {
    pub audio: &'a [u8],
    pub api_key: &'a str,
    pub model: &'a str,
    pub duration_ms: Option<u64>,
    pub keyterms_prompt: Vec<String>,
    pub prompt: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct AssemblyAiTranscribeResult {
    pub text: String,
    pub transcript_id: String,
    pub model: String,
    pub language_code: Option<String>,
    pub audio_duration: Option<f64>,
    pub confidence: Option<f64>,
    pub upload_ms: u64,
    pub submit_ms: u64,
    pub poll_ms: u64,
    pub poll_count: u32,
    pub total_ms: u64,
    pub bytes_sent: u64,
}

#[derive(Debug, Deserialize)]
struct UploadResponse {
    upload_url: String,
}

#[derive(Debug, Serialize)]
struct TranscriptSubmitBody<'a> {
    audio_url: &'a str,
    speech_models: Vec<&'a str>,
    language_detection: bool,
    punctuate: bool,
    format_text: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    keyterms_prompt: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TranscriptResponse {
    id: String,
    status: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    audio_duration: Option<f64>,
    #[serde(default)]
    language_code: Option<String>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    speech_model_used: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    #[serde(default)]
    error: Option<String>,
}

/// Uploads audio bytes, submits a pre-recorded transcription job using
/// `speech_models`, polls until completion, and returns the result.
pub async fn transcribe_audio(
    request: AssemblyAiTranscribeRequest<'_>,
) -> Result<AssemblyAiTranscribeResult, String> {
    let upload_timeout = adaptive_upload_timeout(request.audio.len());
    let poll_timeout = adaptive_poll_timeout(request.duration_ms, request.audio.len());
    transcribe_audio_with_base_url(
        DEFAULT_BASE_URL,
        request,
        upload_timeout,
        Duration::from_secs(30),
        poll_timeout,
    )
    .await
}

async fn transcribe_audio_with_base_url(
    base_url: &str,
    request: AssemblyAiTranscribeRequest<'_>,
    upload_timeout: Duration,
    submit_timeout: Duration,
    poll_timeout: Duration,
) -> Result<AssemblyAiTranscribeResult, String> {
    if request.audio.is_empty() {
        return Err("AssemblyAI: o áudio está vazio.".into());
    }
    let api_key = request.api_key.trim();
    if api_key.is_empty() {
        return Err("Configure uma chave da AssemblyAI em Ajustes → Transcrição.".into());
    }
    let model = request.model.trim();
    if model.is_empty() {
        return Err("AssemblyAI: modelo de transcrição não informado.".into());
    }

    let base = base_url.trim_end_matches('/');
    let total_started = Instant::now();

    // 1. Upload raw audio binary to /v2/upload
    let upload_started = Instant::now();
    let upload_url = upload_audio(base, api_key, request.audio, upload_timeout).await?;
    let upload_ms = upload_started.elapsed().as_millis() as u64;

    // 2. Submit transcript job to /v2/transcript with speech_models
    let submit_started = Instant::now();
    let prompt = request
        .prompt
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let submit_body = TranscriptSubmitBody {
        audio_url: &upload_url,
        speech_models: vec![model],
        language_detection: true,
        punctuate: true,
        format_text: true,
        prompt,
        keyterms_prompt: request.keyterms_prompt,
    };
    let submitted = submit_transcript(base, api_key, &submit_body, submit_timeout).await?;
    let submit_ms = submit_started.elapsed().as_millis() as u64;

    // 3. Poll /v2/transcript/{id} until completed or error
    let poll_started = Instant::now();
    let mut poll_count = 0u32;
    let final_transcript = match submitted.status.as_str() {
        "completed" => submitted,
        "error" => {
            return Err(format!(
                "AssemblyAI falhou ao transcrever: {}",
                submitted
                    .error
                    .unwrap_or_else(|| "erro desconhecido".into())
            ));
        }
        _ => poll_transcript(base, api_key, &submitted.id, poll_timeout, &mut poll_count).await?,
    };
    let poll_ms = poll_started.elapsed().as_millis() as u64;
    let total_ms = total_started.elapsed().as_millis() as u64;

    Ok(AssemblyAiTranscribeResult {
        text: final_transcript.text.unwrap_or_default().trim().to_string(),
        transcript_id: final_transcript.id,
        model: final_transcript
            .speech_model_used
            .filter(|used| !used.trim().is_empty())
            .unwrap_or_else(|| model.to_string()),
        language_code: final_transcript.language_code,
        audio_duration: final_transcript.audio_duration,
        confidence: final_transcript.confidence,
        upload_ms,
        submit_ms,
        poll_ms,
        poll_count,
        total_ms,
        bytes_sent: request.audio.len() as u64,
    })
}

async fn upload_audio(
    base: &str,
    api_key: &str,
    audio: &[u8],
    timeout: Duration,
) -> Result<String, String> {
    let url = format!("{base}/v2/upload");
    let exchange = async {
        let response = http_client()?
            .post(&url)
            .header("Authorization", api_key)
            .header("Content-Type", "application/octet-stream")
            .body(audio.to_vec())
            .send()
            .await
            .map_err(|error| format!("AssemblyAI upload: falha de rede: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("AssemblyAI upload: falha ao ler resposta: {error}"))?;
        Ok::<_, String>((status, body))
    };

    let (status, body) = tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| {
            format!(
                "AssemblyAI upload: timeout após {} segundos.",
                timeout.as_secs()
            )
        })??;

    if !status.is_success() {
        return Err(format_http_error("AssemblyAI upload", status, &body));
    }

    let parsed: UploadResponse = serde_json::from_str(&body)
        .map_err(|error| format!("AssemblyAI upload: resposta inválida: {error}"))?;
    if parsed.upload_url.trim().is_empty() {
        return Err("AssemblyAI upload: upload_url ausente na resposta.".into());
    }
    Ok(parsed.upload_url)
}

async fn submit_transcript(
    base: &str,
    api_key: &str,
    body: &TranscriptSubmitBody<'_>,
    timeout: Duration,
) -> Result<TranscriptResponse, String> {
    let url = format!("{base}/v2/transcript");
    let exchange = async {
        let response = http_client()?
            .post(&url)
            .header("Authorization", api_key)
            .json(body)
            .send()
            .await
            .map_err(|error| format!("AssemblyAI submissão: falha de rede: {error}"))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| format!("AssemblyAI submissão: falha ao ler resposta: {error}"))?;
        Ok::<_, String>((status, text))
    };

    let (status, text) = tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| {
            format!(
                "AssemblyAI submissão: timeout após {} segundos.",
                timeout.as_secs()
            )
        })??;

    if !status.is_success() {
        return Err(format_http_error("AssemblyAI submissão", status, &text));
    }

    let parsed: TranscriptResponse = serde_json::from_str(&text)
        .map_err(|error| format!("AssemblyAI submissão: resposta inválida: {error}"))?;
    if parsed.id.trim().is_empty() {
        return Err("AssemblyAI submissão: ID da transcrição ausente na resposta.".into());
    }
    Ok(parsed)
}

async fn poll_transcript(
    base: &str,
    api_key: &str,
    transcript_id: &str,
    timeout: Duration,
    poll_count: &mut u32,
) -> Result<TranscriptResponse, String> {
    let url = format!("{base}/v2/transcript/{transcript_id}");
    let deadline = Instant::now() + timeout;
    let mut consecutive_transient_errors = 0u32;

    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(format!(
                "AssemblyAI: timeout após {} segundos aguardando transcrição.",
                timeout.as_secs()
            ));
        }

        let delay = poll_backoff_delay(*poll_count);
        let remaining = deadline.saturating_duration_since(now);
        tokio::time::sleep(delay.min(remaining)).await;

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!(
                "AssemblyAI: timeout após {} segundos aguardando transcrição.",
                timeout.as_secs()
            ));
        }

        *poll_count = poll_count.saturating_add(1);
        let request_timeout = remaining.min(Duration::from_secs(20));
        let exchange = async {
            let response = http_client()?
                .get(&url)
                .header("Authorization", api_key)
                .send()
                .await
                .map_err(|error| format!("AssemblyAI polling: falha de rede: {error}"))?;
            let status = response.status();
            let text = response
                .text()
                .await
                .map_err(|error| format!("AssemblyAI polling: falha ao ler resposta: {error}"))?;
            Ok::<_, String>((status, text))
        };

        let response = match tokio::time::timeout(request_timeout, exchange).await {
            Ok(Ok(pair)) => pair,
            Ok(Err(network_error)) => {
                consecutive_transient_errors += 1;
                if consecutive_transient_errors >= 3 {
                    return Err(network_error);
                }
                log::warn!(
                    "assemblyai: erro transitório no polling ({}/3): {}",
                    consecutive_transient_errors,
                    network_error
                );
                continue;
            }
            Err(_) => {
                consecutive_transient_errors += 1;
                if consecutive_transient_errors >= 3 || Instant::now() >= deadline {
                    return Err(format!(
                        "AssemblyAI: timeout após {} segundos aguardando transcrição.",
                        timeout.as_secs()
                    ));
                }
                continue;
            }
        };

        let (status, text) = response;
        if status.is_server_error() || status.as_u16() == 429 {
            consecutive_transient_errors += 1;
            if consecutive_transient_errors >= 3 {
                return Err(format_http_error("AssemblyAI polling", status, &text));
            }
            log::warn!(
                "assemblyai: status transitório {} no polling ({}/3)",
                status.as_u16(),
                consecutive_transient_errors
            );
            continue;
        }
        if !status.is_success() {
            return Err(format_http_error("AssemblyAI polling", status, &text));
        }
        consecutive_transient_errors = 0;

        let parsed: TranscriptResponse = serde_json::from_str(&text)
            .map_err(|error| format!("AssemblyAI polling: resposta inválida: {error}"))?;
        match parsed.status.as_str() {
            "completed" => return Ok(parsed),
            "error" => {
                return Err(format!(
                    "AssemblyAI falhou ao transcrever: {}",
                    parsed.error.unwrap_or_else(|| "erro desconhecido".into())
                ));
            }
            "queued" | "processing" => continue,
            other => {
                log::warn!("assemblyai: status inesperado durante polling: {other}");
            }
        }
    }
}

fn poll_backoff_delay(attempt: u32) -> Duration {
    match attempt {
        0 => Duration::from_millis(300),
        1 => Duration::from_millis(450),
        2 => Duration::from_millis(700),
        3 => Duration::from_millis(1_000),
        4 => Duration::from_millis(1_500),
        _ => Duration::from_millis(2_000),
    }
}

fn format_http_error(stage: &str, status: reqwest::StatusCode, body: &str) -> String {
    let detail = serde_json::from_str::<ErrorEnvelope>(body)
        .ok()
        .and_then(|envelope| envelope.error)
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| truncate_error(body.trim(), 500));
    match status.as_u16() {
        401 | 403 => {
            "AssemblyAI: chave de API inválida ou sem permissão. Verifique em Ajustes → Transcrição."
                .into()
        }
        429 => format!("AssemblyAI: limite de requisições atingido (429): {detail}"),
        code => format!("{stage} retornou status {code}: {detail}"),
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
    use crate::vocabulary::{VocabularyCategory, VocabularyTerm};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn vocab_term(
        canonical: &str,
        aliases: &[&str],
        strict: bool,
        enabled: bool,
    ) -> VocabularyTerm {
        VocabularyTerm {
            canonical: canonical.into(),
            aliases: aliases.iter().map(|s| (*s).into()).collect(),
            category: VocabularyCategory::Other,
            strict,
            enabled,
        }
    }

    #[test]
    fn keyterms_prioritize_strict_exclude_aliases_and_enforce_limits() {
        let terms = vec![
            vocab_term("Tauri", &["taure", "tory"], false, true),
            vocab_term("Sonora", &["som ora"], true, true),
            vocab_term("sonora", &[], false, true),
            vocab_term("Desativado", &[], true, false),
            vocab_term(
                "uma frase muito longa com mais de seis palavras aqui",
                &[],
                true,
                true,
            ),
            vocab_term("linha\nquebrada", &[], true, true),
        ];

        let keyterms = build_keyterms(&terms, MAX_DIRECT_KEYTERMS);
        assert_eq!(keyterms, vec!["Sonora", "Tauri"]);
        assert!(!keyterms.contains(&"taure".to_string()));
        assert!(!keyterms.contains(&"som ora".to_string()));
    }

    #[test]
    fn context_prompt_is_concise_and_avoids_behavioral_instructions() {
        let p = context_prompt();
        assert!(p.split_whitespace().count() <= 15);
        assert!(p.contains("Portuguese"));
        assert!(p.contains("English"));
        assert!(!p.to_ascii_lowercase().contains("do not translate"));
    }

    #[test]
    fn timeouts_scale_with_duration_and_stay_bounded() {
        assert_eq!(
            adaptive_poll_timeout(Some(5_000), 160_000),
            Duration::from_secs(90)
        );
        assert_eq!(
            adaptive_poll_timeout(Some(120_000), 3_840_000),
            Duration::from_secs(420)
        );
        assert_eq!(
            adaptive_poll_timeout(Some(1_000_000), 32_000_000),
            Duration::from_secs(600)
        );
    }

    async fn read_http_request(stream: &mut tokio::net::TcpStream) -> (String, String, Vec<u8>) {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end;
        loop {
            let n = stream.read(&mut tmp).await.unwrap();
            if n == 0 {
                panic!("EOF before headers");
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = pos + 4;
                break;
            }
        }
        let header_str = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let content_length = header_str
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while buf.len() < header_end + content_length {
            let n = stream.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        let first_line = header_str.lines().next().unwrap_or("").to_string();
        let body = buf[header_end..header_end + content_length].to_vec();
        (first_line, header_str, body)
    }

    async fn write_http_response(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(resp.as_bytes()).await.unwrap();
    }

    #[tokio::test]
    async fn direct_prerecorded_flow_uploads_submits_with_speech_models_and_polls() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let poll_hits = Arc::new(AtomicUsize::new(0));
        let captured_submit = Arc::new(Mutex::new(None::<serde_json::Value>));
        let captured_auth = Arc::new(Mutex::new(Vec::<String>::new()));

        let poll_hits_srv = poll_hits.clone();
        let submit_srv = captured_submit.clone();
        let auth_srv = captured_auth.clone();

        let server = tokio::spawn(async move {
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (first_line, headers, body) = read_http_request(&mut stream).await;
                if let Some(auth) = headers
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                {
                    auth_srv.lock().unwrap().push(auth.to_string());
                }
                if first_line.starts_with("POST /v2/upload ") {
                    assert_eq!(body, b"RIFF-synthetic-wav");
                    write_http_response(
                        &mut stream,
                        "200 OK",
                        r#"{"upload_url":"https://cdn.assemblyai.com/upload/mock-1"}"#,
                    )
                    .await;
                } else if first_line.starts_with("POST /v2/transcript ") {
                    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    *submit_srv.lock().unwrap() = Some(parsed);
                    write_http_response(
                        &mut stream,
                        "200 OK",
                        r#"{"id":"tr_123","status":"queued"}"#,
                    )
                    .await;
                } else if first_line.starts_with("GET /v2/transcript/tr_123 ") {
                    let hit = poll_hits_srv.fetch_add(1, Ordering::SeqCst);
                    if hit == 0 {
                        write_http_response(
                            &mut stream,
                            "200 OK",
                            r#"{"id":"tr_123","status":"processing"}"#,
                        )
                        .await;
                    } else {
                        write_http_response(
                            &mut stream,
                            "200 OK",
                            r#"{"id":"tr_123","status":"completed","text":"Abra o deploy do Sonora agora.","audio_duration":3.4,"language_code":"pt","confidence":0.97,"speech_model_used":"assemblyai-stt-3-7"}"#,
                        )
                        .await;
                    }
                } else {
                    panic!("unexpected request: {first_line}");
                }
            }
        });

        let result = transcribe_audio_with_base_url(
            &format!("http://{addr}"),
            AssemblyAiTranscribeRequest {
                audio: b"RIFF-synthetic-wav",
                api_key: "aai-test-key-xyz",
                model: MODEL_STT_3_7,
                duration_ms: Some(3_400),
                keyterms_prompt: vec!["Sonora".into(), "deploy".into()],
                prompt: Some(context_prompt()),
            },
            Duration::from_secs(5),
            Duration::from_secs(5),
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        server.await.unwrap();

        assert_eq!(result.text, "Abra o deploy do Sonora agora.");
        assert_eq!(result.transcript_id, "tr_123");
        assert_eq!(result.model, "assemblyai-stt-3-7");
        assert_eq!(result.language_code.as_deref(), Some("pt"));
        assert_eq!(result.audio_duration, Some(3.4));
        assert_eq!(result.poll_count, 2);

        // Verify Authorization uses raw API key (no Bearer prefix).
        let auths = captured_auth.lock().unwrap();
        assert_eq!(auths.len(), 4);
        for header in auths.iter() {
            assert!(
                header.to_ascii_lowercase().ends_with("aai-test-key-xyz")
                    && !header.to_ascii_lowercase().contains("bearer"),
                "unexpected auth header: {header}"
            );
        }

        // Verify submitted JSON matches AssemblyAI pre-recorded contract.
        let submit = captured_submit.lock().unwrap().clone().unwrap();
        assert_eq!(
            submit["audio_url"],
            "https://cdn.assemblyai.com/upload/mock-1"
        );
        assert_eq!(
            submit["speech_models"],
            serde_json::json!(["assemblyai-stt-3-7"])
        );
        assert_eq!(submit["language_detection"], true);
        assert_eq!(submit["punctuate"], true);
        assert_eq!(submit["format_text"], true);
        assert!(submit.get("speech_threshold").is_none());
        assert_eq!(
            submit["keyterms_prompt"],
            serde_json::json!(["Sonora", "deploy"])
        );
    }

    #[tokio::test]
    async fn auth_failure_and_transcript_error_return_clear_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_http_request(&mut stream).await;
            write_http_response(
                &mut stream,
                "401 Unauthorized",
                r#"{"error":"Authentication error, API token missing/invalid"}"#,
            )
            .await;
        });

        let err = transcribe_audio_with_base_url(
            &format!("http://{addr}"),
            AssemblyAiTranscribeRequest {
                audio: b"RIFF-audio",
                api_key: "bad-key",
                model: MODEL_STT_3_7,
                duration_ms: Some(1_000),
                keyterms_prompt: Vec::new(),
                prompt: None,
            },
            Duration::from_secs(5),
            Duration::from_secs(5),
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        server.await.unwrap();
        assert!(err.contains("chave de API inválida"));
    }

    #[tokio::test]
    async fn completed_empty_transcript_returns_empty_ok_without_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (first_line, _, _) = read_http_request(&mut stream).await;
                if first_line.starts_with("POST /v2/upload ") {
                    write_http_response(
                        &mut stream,
                        "200 OK",
                        r#"{"upload_url":"https://cdn.assemblyai.com/upload/mock-empty"}"#,
                    )
                    .await;
                } else {
                    write_http_response(
                        &mut stream,
                        "200 OK",
                        r#"{"id":"tr_empty","status":"completed","text":""}"#,
                    )
                    .await;
                }
            }
        });

        let result = transcribe_audio_with_base_url(
            &format!("http://{addr}"),
            AssemblyAiTranscribeRequest {
                audio: b"RIFF-silence",
                api_key: "valid-key",
                model: MODEL_STT_3_7,
                duration_ms: Some(4_000),
                keyterms_prompt: Vec::new(),
                prompt: None,
            },
            Duration::from_secs(5),
            Duration::from_secs(5),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert!(result.text.is_empty());
    }
}
