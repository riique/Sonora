//! Direct ElevenLabs Speech-to-Text client (`POST /v1/speech-to-text`).
//!
//! Dictation-oriented defaults: no audio-event tags, no timestamps, filler
//! words removed and numbers as digits. Microphone captures are sent as raw
//! 16 kHz mono PCM (`file_format=pcm_s16le_16`), which ElevenLabs documents as
//! the lower-latency input path. The custom vocabulary becomes `keyterms`.

use reqwest::multipart;
use serde::Deserialize;
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
    time::{Duration, Instant},
};

pub const DEFAULT_BASE_URL: &str = "https://api.elevenlabs.io";
pub const MODEL_SCRIBE_V3: &str = "scribe_v3";
/// Above 100 keyterms ElevenLabs bills a 20-second minimum per request.
pub const MAX_KEYTERMS: usize = 100;
const MAX_KEYTERM_CHARS: usize = 50;
const MAX_KEYTERM_WORDS: usize = 5;
const FORBIDDEN_KEYTERM_CHARS: [char; 7] = ['<', '>', '{', '}', '[', ']', '\\'];

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
/// Set once ElevenLabs rejects `keyterms`, so later dictations skip the retry.
static KEYTERMS_REJECTED: AtomicBool = AtomicBool::new(false);

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
        .ok_or_else(|| "ElevenLabs HTTP client unavailable".into())
}

/// Opens (or refreshes) the pooled TLS connection so the dictation request
/// does not pay the handshake. The response status is irrelevant.
pub async fn warm_connection() {
    let Ok(client) = http_client() else { return };
    let request = client.head(DEFAULT_BASE_URL).send();
    if let Ok(Ok(response)) = tokio::time::timeout(Duration::from_secs(5), request).await {
        let _ = response.bytes().await;
    }
}

/// Vocabulary terms in the shape ElevenLabs accepts: at most 50 characters,
/// 5 words, and none of `< > { } [ ] \`.
pub fn build_keyterms(vocab: &[crate::vocabulary::VocabularyTerm]) -> Vec<String> {
    let mut ordered: Vec<&crate::vocabulary::VocabularyTerm> =
        vocab.iter().filter(|term| term.enabled).collect();
    ordered.sort_by_key(|term| !term.strict);
    let mut seen = HashSet::new();
    ordered
        .into_iter()
        .map(|term| term.canonical.trim())
        .filter(|term| {
            !term.is_empty()
                && term.chars().count() <= MAX_KEYTERM_CHARS
                && term.split_whitespace().count() <= MAX_KEYTERM_WORDS
                && !term
                    .chars()
                    .any(|c| c.is_control() || FORBIDDEN_KEYTERM_CHARS.contains(&c))
        })
        .filter(|term| seen.insert(term.to_lowercase()))
        .take(MAX_KEYTERMS)
        .map(str::to_string)
        .collect()
}

/// Returns the PCM payload of a 16 kHz, mono, 16-bit WAV, or `None` for any
/// other audio (which is then uploaded unchanged).
pub fn pcm_s16le_16k_mono(wav: &[u8]) -> Option<&[u8]> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let mut offset = 12;
    let mut format_ok = false;
    while offset + 8 <= wav.len() {
        let id = &wav[offset..offset + 4];
        let size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().ok()?) as usize;
        let body_start = offset + 8;
        let body_end = body_start.checked_add(size)?;
        if id == b"fmt " {
            let fmt = wav.get(body_start..body_end)?;
            if fmt.len() < 16 {
                return None;
            }
            let audio_format = u16::from_le_bytes([fmt[0], fmt[1]]);
            let channels = u16::from_le_bytes([fmt[2], fmt[3]]);
            let sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
            let bits = u16::from_le_bytes([fmt[14], fmt[15]]);
            format_ok = audio_format == 1 && channels == 1 && sample_rate == 16_000 && bits == 16;
        } else if id == b"data" {
            if !format_ok {
                return None;
            }
            let data = &wav[body_start..body_end.min(wav.len())];
            return (!data.is_empty() && data.len().is_multiple_of(2)).then_some(data);
        }
        // Chunks are word-aligned.
        offset = body_end + (size & 1);
    }
    None
}

#[derive(Debug, Clone)]
pub struct ElevenLabsTranscribeRequest<'a> {
    pub audio: &'a [u8],
    pub ext: &'a str,
    pub api_key: &'a str,
    pub model: &'a str,
    pub keyterms: Vec<String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct ElevenLabsTranscribeResult {
    pub text: String,
    pub model: String,
    pub language_code: Option<String>,
    pub audio_duration_secs: Option<f64>,
    pub transcription_id: Option<String>,
    pub request_ms: u64,
    pub bytes_sent: u64,
    pub sent_pcm: bool,
    pub keyterms_sent: usize,
}

#[derive(Debug, Deserialize)]
struct SpeechToTextResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    language_code: Option<String>,
    #[serde(default)]
    audio_duration_secs: Option<f64>,
    #[serde(default)]
    transcription_id: Option<String>,
}

pub async fn transcribe_audio(
    request: ElevenLabsTranscribeRequest<'_>,
) -> Result<ElevenLabsTranscribeResult, String> {
    transcribe_audio_with_base_url(request, DEFAULT_BASE_URL).await
}

async fn transcribe_audio_with_base_url(
    request: ElevenLabsTranscribeRequest<'_>,
    base_url: &str,
) -> Result<ElevenLabsTranscribeResult, String> {
    if request.audio.is_empty() {
        return Err("ElevenLabs: o áudio está vazio.".into());
    }
    if request.api_key.trim().is_empty() {
        return Err("Configure uma chave da ElevenLabs em Ajustes → Transcrição.".into());
    }
    let keyterms = if KEYTERMS_REJECTED.load(Ordering::Relaxed) {
        Vec::new()
    } else {
        request.keyterms.clone()
    };
    let started = Instant::now();
    match send(&request, base_url, &keyterms).await {
        Err(SendError::Rejected { status, body }) if !keyterms.is_empty() => {
            // A vocabulary problem must never cost the dictation: retry once
            // without keyterms and remember when ElevenLabs named them.
            if body.to_ascii_lowercase().contains("keyterm") {
                KEYTERMS_REJECTED.store(true, Ordering::Relaxed);
            }
            log::warn!(
                "elevenlabs: request with {} keyterms rejected ({status}); retrying without them",
                keyterms.len()
            );
            send(&request, base_url, &[])
                .await
                .map(|out| finish(out, &request, started, 0))
                .map_err(SendError::into_message)
        }
        result => result
            .map(|out| finish(out, &request, started, keyterms.len()))
            .map_err(SendError::into_message),
    }
}

struct SendOutput {
    parsed: SpeechToTextResponse,
    bytes_sent: u64,
    sent_pcm: bool,
}

enum SendError {
    /// 400/422: the request shape was refused (possibly the keyterms).
    Rejected {
        status: u16,
        body: String,
    },
    Other(String),
}

impl SendError {
    fn into_message(self) -> String {
        match self {
            Self::Rejected { status, body } => format_http_error(status, &body),
            Self::Other(message) => message,
        }
    }
}

fn finish(
    out: SendOutput,
    request: &ElevenLabsTranscribeRequest<'_>,
    started: Instant,
    keyterms_sent: usize,
) -> ElevenLabsTranscribeResult {
    ElevenLabsTranscribeResult {
        text: out.parsed.text.trim().to_string(),
        model: request.model.to_string(),
        language_code: out.parsed.language_code,
        audio_duration_secs: out.parsed.audio_duration_secs,
        transcription_id: out.parsed.transcription_id,
        request_ms: started.elapsed().as_millis() as u64,
        bytes_sent: out.bytes_sent,
        sent_pcm: out.sent_pcm,
        keyterms_sent,
    }
}

fn build_form(
    request: &ElevenLabsTranscribeRequest<'_>,
    keyterms: &[String],
) -> Result<(multipart::Form, u64, bool), String> {
    let (payload, file_name, mime, pcm) = match pcm_s16le_16k_mono(request.audio) {
        Some(pcm) => (
            pcm,
            "audio.pcm".to_string(),
            "application/octet-stream",
            true,
        ),
        None => (
            request.audio,
            format!("audio.{}", request.ext.trim_start_matches('.')),
            "application/octet-stream",
            false,
        ),
    };
    let part = multipart::Part::bytes(payload.to_vec())
        .file_name(file_name)
        .mime_str(mime)
        .map_err(|error| format!("ElevenLabs: MIME inválido: {error}"))?;
    let mut form = multipart::Form::new()
        .text("model_id", request.model.to_string())
        .part("file", part)
        .text("tag_audio_events", "false")
        .text("timestamps_granularity", "none")
        .text("diarize", "false")
        .text("no_verbatim", "true");
    if pcm {
        form = form.text("file_format", "pcm_s16le_16");
    }
    if request.model == MODEL_SCRIBE_V3 {
        form = form.text("numbers_format", "digits");
    }
    for term in keyterms {
        form = form.text("keyterms", term.clone());
    }
    Ok((form, payload.len() as u64, pcm))
}

async fn send(
    request: &ElevenLabsTranscribeRequest<'_>,
    base_url: &str,
    keyterms: &[String],
) -> Result<SendOutput, SendError> {
    let (form, bytes_sent, sent_pcm) = build_form(request, keyterms).map_err(SendError::Other)?;
    let url = format!("{}/v1/speech-to-text", base_url.trim_end_matches('/'));
    let exchange = async {
        let response = http_client()
            .map_err(SendError::Other)?
            .post(&url)
            .header("xi-api-key", request.api_key.trim())
            .multipart(form)
            .send()
            .await
            .map_err(|e| SendError::Other(format!("ElevenLabs: falha de rede: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| SendError::Other(format!("ElevenLabs: falha ao ler a resposta: {e}")))?;
        Ok::<_, SendError>((status, body))
    };
    let (status, body) = tokio::time::timeout(request.timeout, exchange)
        .await
        .map_err(|_| {
            SendError::Other(format!(
                "ElevenLabs: timeout após {} segundos.",
                request.timeout.as_secs()
            ))
        })??;
    if !status.is_success() {
        let code = status.as_u16();
        return Err(if matches!(code, 400 | 422) {
            SendError::Rejected { status: code, body }
        } else {
            SendError::Other(format_http_error(code, &body))
        });
    }
    let parsed: SpeechToTextResponse = serde_json::from_str(&body)
        .map_err(|e| SendError::Other(format!("ElevenLabs: resposta inválida: {e}")))?;
    Ok(SendOutput {
        parsed,
        bytes_sent,
        sent_pcm,
    })
}

fn format_http_error(status: u16, body: &str) -> String {
    let detail = truncate_error(body.trim(), 500);
    match status {
        401 => "ElevenLabs: chave de API inválida. Verifique em Ajustes → Transcrição.".into(),
        403 => format!("ElevenLabs: acesso negado (403): {detail}"),
        429 => format!("ElevenLabs: limite de uso ou de requisições atingido (429): {detail}"),
        code => format!("ElevenLabs retornou status {code}: {detail}"),
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
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn term(canonical: &str, strict: bool, enabled: bool) -> VocabularyTerm {
        VocabularyTerm {
            canonical: canonical.into(),
            aliases: vec!["alias errado".into()],
            category: VocabularyCategory::Other,
            strict,
            enabled,
        }
    }

    #[test]
    fn keyterms_respect_elevenlabs_limits() {
        let vocab = vec![
            term("Tauri", false, true),
            term("Sonora", true, true),
            term("sonora", false, true),
            term("desligado", false, false),
            term("um dois três quatro cinco seis", false, true),
            term("[colchetes]", false, true),
            term(&"x".repeat(51), false, true),
        ];
        assert_eq!(build_keyterms(&vocab), vec!["Sonora", "Tauri"]);

        let many: Vec<_> = (0..150)
            .map(|i| term(&format!("termo{i}"), false, true))
            .collect();
        assert_eq!(build_keyterms(&many).len(), MAX_KEYTERMS);
    }

    #[test]
    fn extracts_pcm_only_from_16k_mono_16bit_wav() {
        let samples = [1i16, -2, 3, -4];
        let wav = crate::audio::create_wav_buffer(&samples);
        let pcm = pcm_s16le_16k_mono(&wav).expect("Sonora WAV is 16 kHz mono PCM");
        assert_eq!(pcm, &[1, 0, 254, 255, 3, 0, 252, 255]);

        let mut stereo = wav.clone();
        stereo[22] = 2;
        assert!(pcm_s16le_16k_mono(&stereo).is_none());
        assert!(pcm_s16le_16k_mono(b"ID3 mp3 bytes").is_none());
    }

    async fn read_request(stream: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end = loop {
            let n = stream.read(&mut tmp).await.unwrap();
            assert!(n > 0, "EOF before headers");
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .expect("multipart body has a known length");
        while buf.len() < header_end + length {
            let n = stream.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        (headers, buf[header_end..header_end + length].to_vec())
    }

    async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(resp.as_bytes()).await.unwrap();
    }

    fn field(body: &str, name: &str) -> Vec<String> {
        let marker = format!("name=\"{name}\"\r\n\r\n");
        body.match_indices(&marker)
            .map(|(at, _)| {
                let rest = &body[at + marker.len()..];
                rest[..rest.find("\r\n").unwrap_or(rest.len())].to_string()
            })
            .collect()
    }

    #[tokio::test]
    async fn sends_pcm_with_dictation_defaults_and_retries_without_rejected_keyterms() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let bodies = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let bodies_srv = bodies.clone();
        let server = tokio::spawn(async move {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (headers, body) = read_request(&mut stream).await;
                bodies_srv
                    .lock()
                    .unwrap()
                    .push((headers, String::from_utf8_lossy(&body).to_string()));
                if attempt == 0 {
                    respond(
                        &mut stream,
                        "422 Unprocessable Entity",
                        r#"{"detail":[{"loc":["body","keyterms"],"msg":"invalid keyterm","type":"value_error"}]}"#,
                    )
                    .await;
                } else {
                    respond(
                        &mut stream,
                        "200 OK",
                        r#"{"language_code":"por","language_probability":0.99,"text":" Olá, Sonora. ","words":[],"audio_duration_secs":1.5,"transcription_id":"tr-1"}"#,
                    )
                    .await;
                }
            }
        });

        let wav = crate::audio::create_wav_buffer(&[10i16, 20, 30, 40]);
        let out = transcribe_audio_with_base_url(
            ElevenLabsTranscribeRequest {
                audio: &wav,
                ext: "wav",
                api_key: " el-key ",
                model: MODEL_SCRIBE_V3,
                keyterms: vec!["Sonora".into(), "Tauri".into()],
                timeout: Duration::from_secs(10),
            },
            &format!("http://{addr}"),
        )
        .await
        .expect("retry without keyterms succeeds");
        server.await.unwrap();

        assert_eq!(out.text, "Olá, Sonora.");
        assert_eq!(out.language_code.as_deref(), Some("por"));
        assert_eq!(out.transcription_id.as_deref(), Some("tr-1"));
        assert!(out.sent_pcm);
        assert_eq!(out.bytes_sent, 8);
        assert_eq!(out.keyterms_sent, 0);

        let bodies = bodies.lock().unwrap();
        let (headers, first) = &bodies[0];
        assert!(headers.starts_with("POST /v1/speech-to-text "));
        assert!(headers.to_ascii_lowercase().contains("xi-api-key: el-key"));
        assert_eq!(field(first, "model_id"), vec!["scribe_v3"]);
        assert_eq!(field(first, "file_format"), vec!["pcm_s16le_16"]);
        assert_eq!(field(first, "tag_audio_events"), vec!["false"]);
        assert_eq!(field(first, "timestamps_granularity"), vec!["none"]);
        assert_eq!(field(first, "no_verbatim"), vec!["true"]);
        assert_eq!(field(first, "numbers_format"), vec!["digits"]);
        assert_eq!(field(first, "keyterms"), vec!["Sonora", "Tauri"]);
        assert!(
            !first.contains("RIFF"),
            "WAV header must not be uploaded as PCM"
        );
        assert!(field(&bodies[1].1, "keyterms").is_empty());
        assert!(KEYTERMS_REJECTED.load(Ordering::Relaxed));
        KEYTERMS_REJECTED.store(false, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn auth_failure_is_reported_without_retry() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut stream).await;
            respond(
                &mut stream,
                "401 Unauthorized",
                r#"{"detail":{"status":"invalid_api_key"}}"#,
            )
            .await;
        });
        let error = transcribe_audio_with_base_url(
            ElevenLabsTranscribeRequest {
                audio: b"ID3 not wav",
                ext: "mp3",
                api_key: "bad",
                model: MODEL_SCRIBE_V3,
                keyterms: Vec::new(),
                timeout: Duration::from_secs(10),
            },
            &format!("http://{addr}"),
        )
        .await
        .unwrap_err();
        server.await.unwrap();
        assert!(error.contains("chave de API inválida"), "{error}");
    }
}
