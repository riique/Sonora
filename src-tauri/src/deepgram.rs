//! Deepgram Nova-3 transcription client.
//!
//! Supports two transport modes (see [`crate::models::DeepgramMode`]):
//!
//! * **Batch** — sends the full in-memory audio buffer via HTTP REST
//!   (`POST /v1/listen`) and returns the complete transcript.
//! * **Streaming Final Only** — WebSocket streaming with
//!   `interim_results=false`. Partials never leave this module.
//!
//! For **microphone** capture in `streaming_final` mode the preferred path is
//! **live streaming during recording** ([`spawn_live_session`]): PCM is pushed
//! in ~50 ms frames while the user speaks, so when they stop only a
//! residual + `CloseStream` flush remains. That is dramatically faster than
//! re-uploading the full file after stop (and competitive with Groq Whisper).
//!
//! For **file uploads** or **live-session failure**, prefer **batch REST**
//! (pre-recorded API). WebSocket post-hoc remains only as a last-resort path.
//!
//! Both modes return the same type of raw acoustic string so the rest of
//! the pipeline (sanitizer → clipboard → history) is mode-agnostic.
//!
//! The request is configured with a K-Term (`Sonora`) that biases the
//! acoustic model toward the product proper noun.

use crate::models::DeepgramMode;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{sync::OnceLock, time::Duration};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};

/// Base URL for the Deepgram HTTP transcription API. Query parameters
/// carrying the model, feature flags and the K-Term are appended by
/// [`build_batch_request_url`].
const DEEPGRAM_BASE_URL: &str = "https://api.deepgram.com/v1/listen";

/// WebSocket base for the Deepgram live streaming API.
const DEEPGRAM_WS_BASE_URL: &str = "wss://api.deepgram.com/v1/listen";

/// K-Term injected into every Deepgram request. Deepgram weights the
/// supplied term higher during decoding, which measurably reduces
/// substitution errors on the "Sonora" proper noun.
const KEYTERM: &str = "Sonora";

/// Soft timeout for the HTTP batch exchange. Deepgram typically answers a
/// short clip in well under five seconds, so thirty seconds gives ample
/// headroom for longer recordings without letting the request hang
/// forever on a degraded network.
const BATCH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Hard deadline for a full streaming session (connect + send + final wait).
const STREAMING_SESSION_TIMEOUT: Duration = Duration::from_secs(90);

/// Hard deadline to wait for the server after `CloseStream` (Metadata/close).
const STREAMING_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// WebSocket handshake hard timeout (soft target is ~1.5 s).
const STREAMING_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Fallback chunk size when sample rate is unknown (~50 ms @ 16 kHz mono 16-bit).
const STREAM_CHUNK_BYTES_DEFAULT: usize = 1_600;

/// One automatic reconnect after a transport-level failure (post-hoc only).
const STREAM_MAX_ATTEMPTS: u32 = 2;

/// Ideal media chunk duration for live / post-hoc streaming.
/// Deepgram recommends 20–100 ms buffers; 50 ms balances tail latency and frame rate.
const STREAM_CHUNK_MS: u32 = 50;

/// Commands from the capture thread / stop path into the live WebSocket task.
enum LiveCmd {
    /// Little-endian linear16 mono PCM bytes.
    Pcm(Vec<u8>),
    /// User stopped recording — flush and return the transcript.
    Finish,
    /// User cancelled — close without caring about the transcript.
    Cancel,
}

/// Handle for a Deepgram live streaming session started at record-start.
///
/// The capture callback pushes mono PCM; `finish` / `abort` end the session.
/// Safe to call `push_mono_i16` from the real-time audio thread (non-blocking
/// unbounded send).
pub struct DeepgramLiveSession {
    tx: mpsc::UnboundedSender<LiveCmd>,
    result_rx: Option<oneshot::Receiver<Result<String, String>>>,
    /// How many mono i16 samples have already been pushed (for catch-up on stop).
    sent_samples: AtomicUsize,
}

impl DeepgramLiveSession {
    /// Push mono i16 samples (native capture rate). Non-blocking.
    pub fn push_mono_i16(&self, samples: &[i16]) {
        if samples.is_empty() {
            return;
        }
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for &s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.sent_samples
            .fetch_add(samples.len(), Ordering::Relaxed);
        let _ = self.tx.send(LiveCmd::Pcm(bytes));
    }

    /// Push any buffer suffix not yet sent (covers the race between setting
    /// `recording=false` and draining the mic stream).
    pub fn catch_up_from_buffer(&self, full_mono: &[i16]) {
        let sent = self.sent_samples.load(Ordering::Relaxed);
        if sent < full_mono.len() {
            self.push_mono_i16(&full_mono[sent..]);
        }
    }

    /// Signal end-of-audio and await the final transcript (only finals).
    pub async fn finish(mut self) -> Result<String, String> {
        let _ = self.tx.send(LiveCmd::Finish);
        let rx = self
            .result_rx
            .take()
            .ok_or_else(|| "deepgram live: result already consumed".to_string())?;
        match tokio::time::timeout(STREAMING_DRAIN_TIMEOUT + Duration::from_secs(5), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("deepgram live: session task dropped".to_string()),
            Err(_) => Err("deepgram live: timed out waiting for final transcript".to_string()),
        }
    }

    /// Abandon the session (cancel / panic shortcut). Best-effort.
    pub fn abort(self) {
        let _ = self.tx.send(LiveCmd::Cancel);
        // Drop result_rx — the task will exit on Cancel without blocking us.
    }
}

/// Spawns a background live WebSocket session at the device's native sample
/// rate. Returns immediately; the handshake runs on the Tokio runtime.
pub fn spawn_live_session(api_key: String, sample_rate: u32) -> DeepgramLiveSession {
    let (tx, rx) = mpsc::unbounded_channel::<LiveCmd>();
    let (result_tx, result_rx) = oneshot::channel();

    tauri::async_runtime::spawn(async move {
        let outcome = run_live_session(rx, &api_key, sample_rate).await;
        match &outcome {
            Ok(t) => log::info!(
                "deepgram live: session completed ({} chars) @ {} Hz",
                t.len(),
                sample_rate
            ),
            Err(e) => log::warn!("deepgram live: session failed: {}", e),
        }
        let _ = result_tx.send(outcome);
    });

    DeepgramLiveSession {
        tx,
        result_rx: Some(result_rx),
        sent_samples: AtomicUsize::new(0),
    }
}

fn chunk_bytes_for_rate(sample_rate: u32) -> usize {
    // mono i16: sample_rate * 2 bytes/s * chunk_ms / 1000
    // 50 ms @ 16 kHz = 1_600 B; @ 48 kHz = 4_800 B
    let n = (sample_rate as usize)
        .saturating_mul(2)
        .saturating_mul(STREAM_CHUNK_MS as usize)
        / 1000;
    // Keep within the official ~20–100 ms envelope even if rate is odd.
    n.clamp(640, 9_600)
}

async fn run_live_session(
    mut rx: mpsc::UnboundedReceiver<LiveCmd>,
    api_key: &str,
    sample_rate: u32,
) -> Result<String, String> {
    let t0 = std::time::Instant::now();
    let url = build_streaming_url(sample_rate);

    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("deepgram live: invalid request: {}", e))?;
    let token = format!("Token {}", api_key);
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&token)
            .map_err(|e| format!("deepgram live: invalid auth header: {}", e))?,
    );

    let (ws_stream, _) = tokio::time::timeout(STREAMING_CONNECT_TIMEOUT, connect_async(request))
        .await
        .map_err(|_| {
            format!(
                "deepgram live: connect timed out after {}s",
                STREAMING_CONNECT_TIMEOUT.as_secs()
            )
        })?
        .map_err(|e| format!("deepgram live: websocket connect failed: {}", e))?;

    log::info!(
        "deepgram live: connected in {}ms @ {} Hz",
        t0.elapsed().as_millis(),
        sample_rate
    );

    let (mut write, mut read) = ws_stream.split();
    let chunk_target = chunk_bytes_for_rate(sample_rate);

    let collector = tokio::spawn(async move {
        let mut segments: Vec<String> = Vec::new();
        let mut server_error: Option<String> = None;
        while let Some(frame) = read.next().await {
            let msg = match frame {
                Ok(m) => m,
                Err(e) => {
                    if segments.is_empty() && server_error.is_none() {
                        server_error = Some(format!("websocket error: {}", e));
                    }
                    break;
                }
            };
            match msg {
                Message::Text(text) => match handle_streaming_text(&text, &mut segments) {
                    Ok(StreamingEvent::Results) | Ok(StreamingEvent::Ignore) => {}
                    Ok(StreamingEvent::Metadata) => break,
                    Err(e) => {
                        server_error = Some(e);
                        break;
                    }
                },
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
            }
        }
        (segments, server_error)
    });

    let mut pending: Vec<u8> = Vec::with_capacity(chunk_target * 2);
    let mut finished = false;
    let mut bytes_sent: usize = 0;
    let mut frames_sent: usize = 0;
    let mut queue_peak_bytes: usize = 0;

    while let Some(cmd) = rx.recv().await {
        match cmd {
            LiveCmd::Pcm(bytes) => {
                pending.extend_from_slice(&bytes);
                if pending.len() > queue_peak_bytes {
                    queue_peak_bytes = pending.len();
                }
                while pending.len() >= chunk_target {
                    let chunk: Vec<u8> = pending.drain(..chunk_target).collect();
                    let n = chunk.len();
                    write
                        .send(Message::Binary(chunk.into()))
                        .await
                        .map_err(|e| format!("deepgram live: send chunk failed: {}", e))?;
                    bytes_sent += n;
                    frames_sent += 1;
                }
            }
            LiveCmd::Finish => {
                finished = true;
                break;
            }
            LiveCmd::Cancel => {
                let _ = write
                    .send(Message::Text(r#"{"type":"CloseStream"}"#.into()))
                    .await;
                let _ = write.close().await;
                // Abort collector without waiting for a transcript.
                collector.abort();
                return Err("deepgram live: cancelled".to_string());
            }
        }
    }

    if !finished {
        // Channel closed without Finish (caller dropped) — treat as cancel.
        let _ = write
            .send(Message::Text(r#"{"type":"CloseStream"}"#.into()))
            .await;
        let _ = write.close().await;
        collector.abort();
        return Err("deepgram live: session dropped".to_string());
    }

    // Flush residual (do not wait to fill a full 50 ms frame) then CloseStream.
    // CloseStream alone forces pending audio processing and ends the session;
    // Finalize is reserved for persistent multi-utterance sockets.
    let residual_bytes = pending.len();
    if !pending.is_empty() {
        let residual = std::mem::take(&mut pending);
        write
            .send(Message::Binary(residual.into()))
            .await
            .map_err(|e| format!("deepgram live: send final chunk failed: {}", e))?;
        bytes_sent += residual_bytes;
        frames_sent += 1;
    }

    let flush_start = std::time::Instant::now();
    write
        .send(Message::Text(r#"{"type":"CloseStream"}"#.into()))
        .await
        .map_err(|e| format!("deepgram live: CloseStream failed: {}", e))?;

    let drain = tokio::time::timeout(STREAMING_DRAIN_TIMEOUT, collector).await;
    let _ = write.close().await;

    let (segments, server_error) = match drain {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            return Err(format!("deepgram live: collector failed: {}", e));
        }
        Err(_) => {
            return Err(format!(
                "deepgram live: drain timed out after {}ms",
                STREAMING_DRAIN_TIMEOUT.as_millis()
            ));
        }
    };

    if let Some(err) = server_error {
        if segments.is_empty() {
            return Err(format!("deepgram live: {}", err));
        }
        log::warn!("deepgram live: server error after partial success: {}", err);
    }

    let transcript = join_segments(&segments);
    let queue_peak_ms = if sample_rate > 0 {
        (queue_peak_bytes as u64 * 1000) / (sample_rate as u64 * 2)
    } else {
        0
    };
    log::info!(
        "deepgram live: flush+drain {}ms (residual {} B), total session {}ms, \
         bytes_sent={} frames={} queue_peak={}B (~{}ms), chunk_target={}B ({}ms), transcript {} chars",
        flush_start.elapsed().as_millis(),
        residual_bytes,
        t0.elapsed().as_millis(),
        bytes_sent,
        frames_sent,
        queue_peak_bytes,
        queue_peak_ms,
        chunk_target,
        STREAM_CHUNK_MS,
        transcript.len()
    );
    Ok(transcript)
}

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn http_client() -> Result<&'static reqwest::Client, String> {
    if let Some(client) = HTTP_CLIENT.get() {
        return Ok(client);
    }
    let client = reqwest::Client::builder()
        .timeout(BATCH_REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let _ = HTTP_CLIENT.set(client);
    HTTP_CLIENT
        .get()
        .ok_or_else(|| "failed to initialize http client".to_string())
}

// ─── Shared response shapes ─────────────────────────────────────────────────

/// Root deserialization target for the Deepgram **batch** JSON response.
///
/// Mirrors the documented contract:
///   `results` -> `channels[]` -> `alternatives[]` -> `transcript`
#[derive(Debug, Deserialize)]
struct DeepgramBatchResponse {
    results: DeepgramResults,
}

#[derive(Debug, Deserialize)]
struct DeepgramResults {
    channels: Vec<DeepgramChannel>,
}

#[derive(Debug, Deserialize)]
struct DeepgramChannel {
    alternatives: Vec<DeepgramAlternative>,
}

#[derive(Debug, Deserialize)]
struct DeepgramAlternative {
    transcript: Option<String>,
}

/// Streaming WebSocket message envelope. Extra fields are ignored so the
/// parser stays tolerant across Deepgram model revisions.
#[derive(Debug, Deserialize)]
struct StreamingMessage {
    #[serde(default)]
    #[serde(rename = "type")]
    msg_type: Option<String>,
    #[serde(default)]
    is_final: Option<bool>,
    #[serde(default)]
    speech_final: Option<bool>,
    #[serde(default)]
    channel: Option<StreamingChannel>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamingChannel {
    alternatives: Vec<DeepgramAlternative>,
}

// ─── Public entry point ─────────────────────────────────────────────────────

/// Dispatches audio to Deepgram using the selected [`DeepgramMode`].
///
/// Returns the raw (un-sanitised) transcript string. Downstream stages
/// (sanitizer, history, clipboard) are intentionally unaware of the mode.
pub async fn transcribe(
    audio_bytes: Vec<u8>,
    content_type: &str,
    api_key: &str,
    mode: DeepgramMode,
) -> Result<String, String> {
    transcribe_with_keyterms(audio_bytes, content_type, api_key, mode, &[]).await
}

/// Like [`transcribe`] with extra vocabulary keyterms (batch REST primarily).
pub async fn transcribe_with_keyterms(
    audio_bytes: Vec<u8>,
    content_type: &str,
    api_key: &str,
    mode: DeepgramMode,
    extra_keyterms: &[String],
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err("deepgram api key is missing or empty".to_string());
    }

    match mode {
        DeepgramMode::Batch => {
            log::info!(
                "deepgram: mode=batch, bytes={}, extra_keyterms={}",
                audio_bytes.len(),
                extra_keyterms.len()
            );
            call_deepgram_api_with_keyterms(audio_bytes, content_type, api_key, extra_keyterms)
                .await
        }
        DeepgramMode::StreamingFinal => {
            log::info!(
                "deepgram: mode=streaming_final, bytes={}, content_type={}",
                audio_bytes.len(),
                content_type
            );
            // Streaming URL keeps product keyterm; extras applied on batch fallback.
            call_deepgram_streaming_final(audio_bytes, content_type, api_key).await
        }
    }
}

// ─── Batch (REST) ───────────────────────────────────────────────────────────

/// Assembles the fully-qualified Deepgram batch request URL with the
/// latência-previsível profile (pt-BR fixed, no smart_format / measurements).
///
/// `extra_keyterms` are optional user vocabulary canonicals (in addition to
/// the built-in product keyterm "Sonora").
fn build_batch_request_url(extra_keyterms: &[String]) -> String {
    let mut params = vec![
        "model=nova-3".to_string(),
        "language=pt-BR".to_string(),
        "punctuate=true".to_string(),
        "numerals=true".to_string(),
        "paragraphs=false".to_string(),
        format!("keyterm={}", KEYTERM),
    ];
    // Cap extras to keep the URL reasonable; Deepgram accepts repeated keyterm=.
    for t in extra_keyterms.iter().take(20) {
        let cleaned: String = t
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
            .collect();
        if !cleaned.is_empty() && !cleaned.eq_ignore_ascii_case(KEYTERM) {
            params.push(format!("keyterm={}", urlencoding_minimal(&cleaned)));
        }
    }
    format!("{}?{}", DEEPGRAM_BASE_URL, params.join("&"))
}

/// Minimal query escape (space → %20); avoids pulling a full urlencoding crate.
fn urlencoding_minimal(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            c if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') => c.to_string(),
            c => format!("%{:02X}", c as u8),
        })
        .collect()
}

/// Sends the in-memory audio buffer to the Deepgram `nova-3` HTTP endpoint.
///
/// Microphone captures pass `audio/wav`; uploads pass the MIME detected from
/// the file extension. Deepgram also auto-detects most containers, but sending
/// the correct content type avoids ambiguity.
pub async fn call_deepgram_api(
    audio_bytes: Vec<u8>,
    content_type: &str,
    api_key: &str,
) -> Result<String, String> {
    call_deepgram_api_with_keyterms(audio_bytes, content_type, api_key, &[]).await
}

/// Like [`call_deepgram_api`] but injects extra vocabulary keyterms.
pub async fn call_deepgram_api_with_keyterms(
    audio_bytes: Vec<u8>,
    content_type: &str,
    api_key: &str,
    extra_keyterms: &[String],
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err("deepgram api key is missing or empty".to_string());
    }

    let url = build_batch_request_url(extra_keyterms);
    let client = http_client()?;

    let response = client
        .post(url)
        .header("Authorization", format!("Token {}", api_key))
        .header("Content-Type", content_type)
        .body(audio_bytes)
        .send()
        .await
        .map_err(|e| format!("deepgram network request failed: {}", e))?;

    let status = response.status().as_u16();

    if status != 200 {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "deepgram api returned non-success status {}: {}",
            status, body
        ));
    }

    let parsed: DeepgramBatchResponse = response
        .json()
        .await
        .map_err(|e| format!("failed to parse deepgram response json: {}", e))?;

    let transcript = parsed
        .results
        .channels
        .into_iter()
        .next()
        .and_then(|c| c.alternatives.into_iter().next())
        .and_then(|a| a.transcript)
        .ok_or_else(|| "deepgram response did not contain a transcript field".to_string())?;

    Ok(transcript.trim().to_string())
}

// ─── Streaming Final Only (WebSocket) ───────────────────────────────────────

/// Builds the WebSocket URL with the latência-previsível streaming profile.
/// `sample_rate` is the native capture rate for live sessions, or 16000 for
/// post-hoc 16 kHz WAV PCM.
fn build_streaming_url(sample_rate: u32) -> String {
    let keyterm = format!("keyterm={}", KEYTERM);
    let rate = format!("sample_rate={}", sample_rate);
    // Recommended hot-path profile (Otimization.txt §4):
    // language=pt-BR fixed; no smart_format / no_delay / measurements /
    // detect_language / vad_events / utterance_end_ms. Sanitizer owns final form.
    let params: [&str; 11] = [
        "model=nova-3",
        "language=pt-BR",
        "encoding=linear16",
        rate.as_str(),
        "channels=1",
        "interim_results=false",
        "endpointing=300",
        "punctuate=true",
        "numerals=true",
        "paragraphs=false",
        keyterm.as_str(),
    ];
    format!("{}?{}", DEEPGRAM_WS_BASE_URL, params.join("&"))
}

/// Post-hoc transcription of a recorded audio buffer.
///
/// Prefers **batch REST** (pre-recorded API) for complete files. WebSocket
/// post-hoc loses the live-stream advantage and is only used if batch fails
/// and the payload is linear16 PCM.
async fn call_deepgram_streaming_final(
    audio_bytes: Vec<u8>,
    content_type: &str,
    api_key: &str,
) -> Result<String, String> {
    // Primary path for complete audio: batch REST (not WS file-blast).
    match call_deepgram_api(audio_bytes.clone(), content_type, api_key).await {
        Ok(text) => {
            log::info!(
                "deepgram streaming_final post-hoc: batch REST ok ({} chars)",
                text.len()
            );
            Ok(text)
        }
        Err(batch_err) => {
            log::warn!(
                "deepgram streaming_final post-hoc: batch REST failed ({}); \
                 trying WS post-hoc if PCM available",
                batch_err
            );
            let pcm = match extract_linear16_pcm(&audio_bytes, content_type) {
                Some(pcm) if !pcm.is_empty() => pcm,
                _ => return Err(batch_err),
            };

            let mut last_err = batch_err;
            for attempt in 1..=STREAM_MAX_ATTEMPTS {
                match streaming_session_once(&pcm, api_key).await {
                    Ok(text) => {
                        log::info!(
                            "deepgram streaming_final post-hoc: WS ok on attempt {} ({} chars)",
                            attempt,
                            text.len()
                        );
                        return Ok(text);
                    }
                    Err(e) => {
                        let retryable = is_retryable_stream_error(&e);
                        log::warn!(
                            "deepgram streaming_final post-hoc WS attempt {}/{} \
                             failed (retryable={}): {}",
                            attempt,
                            STREAM_MAX_ATTEMPTS,
                            retryable,
                            e
                        );
                        last_err = e;
                        if !retryable || attempt == STREAM_MAX_ATTEMPTS {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(250 * attempt as u64)).await;
                    }
                }
            }
            Err(last_err)
        }
    }
}

fn is_retryable_stream_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("connect")
        || lower.contains("handshake")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("network")
        || lower.contains("temporarily")
        || lower.contains("websocket error")
}

/// Single streaming attempt with a hard session deadline.
async fn streaming_session_once(pcm: &[u8], api_key: &str) -> Result<String, String> {
    tokio::time::timeout(
        STREAMING_SESSION_TIMEOUT,
        streaming_session_inner(pcm, api_key),
    )
    .await
    .map_err(|_| {
        format!(
            "deepgram streaming session timed out after {}s",
            STREAMING_SESSION_TIMEOUT.as_secs()
        )
    })?
}

async fn streaming_session_inner(pcm: &[u8], api_key: &str) -> Result<String, String> {
    // Post-hoc path always feeds 16 kHz PCM extracted from our WAV encoder.
    let sample_rate = 16_000u32;
    let chunk_size = chunk_bytes_for_rate(sample_rate).max(STREAM_CHUNK_BYTES_DEFAULT);
    let url = build_streaming_url(sample_rate);

    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("deepgram streaming: invalid request: {}", e))?;

    let token = format!("Token {}", api_key);
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&token)
            .map_err(|e| format!("deepgram streaming: invalid auth header: {}", e))?,
    );

    let (ws_stream, _response) =
        tokio::time::timeout(STREAMING_CONNECT_TIMEOUT, connect_async(request))
            .await
            .map_err(|_| {
                format!(
                    "deepgram streaming: connect timed out after {}ms",
                    STREAMING_CONNECT_TIMEOUT.as_millis()
                )
            })?
            .map_err(|e| format!("deepgram streaming: websocket connect failed: {}", e))?;

    log::info!(
        "deepgram streaming_final (post-hoc WS): connected, pcm_bytes={}, chunk_size={} ({}ms)",
        pcm.len(),
        chunk_size,
        STREAM_CHUNK_MS
    );

    let (mut write, mut read) = ws_stream.split();

    // ── Concurrent reader: accumulate final transcripts while we send ──
    let collector = tokio::spawn(async move {
        let mut segments: Vec<String> = Vec::new();
        let mut saw_metadata = false;
        let mut server_error: Option<String> = None;

        while let Some(frame) = read.next().await {
            let msg = match frame {
                Ok(m) => m,
                Err(e) => {
                    if segments.is_empty() && server_error.is_none() {
                        server_error = Some(format!("websocket error: {}", e));
                    }
                    break;
                }
            };

            match msg {
                Message::Text(text) => match handle_streaming_text(&text, &mut segments) {
                    Ok(StreamingEvent::Results) => {}
                    Ok(StreamingEvent::Metadata) => {
                        saw_metadata = true;
                        break;
                    }
                    Ok(StreamingEvent::Ignore) => {}
                    Err(e) => {
                        server_error = Some(e);
                        break;
                    }
                },
                Message::Binary(_) => {}
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Close(_) => break,
                Message::Frame(_) => {}
            }
        }

        (segments, saw_metadata, server_error)
    });

    // ── Push PCM chunks as fast as the socket allows (no artificial sleep) ──
    let mut offset = 0usize;
    while offset < pcm.len() {
        let end = (offset + chunk_size).min(pcm.len());
        let chunk = pcm[offset..end].to_vec();
        write
            .send(Message::Binary(chunk.into()))
            .await
            .map_err(|e| format!("deepgram streaming: failed to send audio chunk: {}", e))?;
        offset = end;
    }

    log::info!(
        "deepgram streaming_final (post-hoc WS): all {} pcm bytes sent, CloseStream",
        pcm.len()
    );

    // Disposable connection: CloseStream alone flushes and ends the session.
    write
        .send(Message::Text(r#"{"type":"CloseStream"}"#.into()))
        .await
        .map_err(|e| format!("deepgram streaming: failed to send CloseStream: {}", e))?;

    let drain = tokio::time::timeout(STREAMING_DRAIN_TIMEOUT, collector).await;
    let _ = write.close().await;

    let (segments, _saw_metadata, server_error) = match drain {
        Ok(join_res) => {
            join_res.map_err(|e| format!("deepgram streaming: collector task failed: {}", e))?
        }
        Err(_) => {
            return Err(format!(
                "deepgram streaming: timed out waiting for final results ({}ms)",
                STREAMING_DRAIN_TIMEOUT.as_millis()
            ));
        }
    };

    if let Some(err) = server_error {
        if segments.is_empty() {
            return Err(format!("deepgram streaming: {}", err));
        }
        log::warn!(
            "deepgram streaming_final: server error after partial success: {}",
            err
        );
    }

    let transcript = join_segments(&segments);
    if transcript.is_empty() {
        log::info!("deepgram streaming_final: empty transcript (silence or no speech)");
    }

    Ok(transcript)
}

enum StreamingEvent {
    Results,
    Metadata,
    Ignore,
}

/// Parses one WebSocket text frame. Appends non-empty final transcripts to
/// `segments`. Application-level errors become `Err`.
fn handle_streaming_text(text: &str, segments: &mut Vec<String>) -> Result<StreamingEvent, String> {
    let msg: StreamingMessage = serde_json::from_str(text).map_err(|e| {
        format!(
            "failed to parse streaming message: {} (payload starts with {:?})",
            e,
            text.chars().take(80).collect::<String>()
        )
    })?;

    let msg_type = msg.msg_type.as_deref().unwrap_or("");

    match msg_type {
        "Results" => {
            // With interim_results=false every Results message is final, but
            // we still gate on is_final for defensive correctness.
            let is_final = msg.is_final.unwrap_or(true);
            if !is_final {
                return Ok(StreamingEvent::Ignore);
            }

            let piece = msg
                .channel
                .as_ref()
                .and_then(|c| c.alternatives.first())
                .and_then(|a| a.transcript.as_ref())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());

            if let Some(t) = piece {
                // Prefer speech_final segments when available, but always keep
                // is_final text — concatenating is_final pieces reconstructs
                // the full utterance even when speech_final is delayed.
                let speech_final = msg.speech_final.unwrap_or(false);
                log::debug!(
                    "deepgram streaming_final: segment is_final=true speech_final={} len={}",
                    speech_final,
                    t.len()
                );
                segments.push(t.to_string());
            }
            Ok(StreamingEvent::Results)
        }
        "Metadata" => Ok(StreamingEvent::Metadata),
        "Error" | "error" => {
            let detail = msg
                .error
                .or(msg.description)
                .or(msg.message)
                .unwrap_or_else(|| text.to_string());
            Err(format!("server error: {}", detail))
        }
        // UtteranceEnd / SpeechStarted / etc. — ignore for final-only mode.
        _ => Ok(StreamingEvent::Ignore),
    }
}

fn join_segments(segments: &[String]) -> String {
    segments
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ─── PCM / WAV helpers ──────────────────────────────────────────────────────

/// Attempts to obtain raw little-endian linear16 mono PCM from a recorded
/// buffer. Returns `None` when the payload is a compressed container (mp3,
/// m4a, …) so the caller can fall back to batch REST (Deepgram auto-detects
/// those formats over HTTP).
fn extract_linear16_pcm(audio_bytes: &[u8], content_type: &str) -> Option<Vec<u8>> {
    let ct = content_type.to_ascii_lowercase();
    let looks_wav = ct.contains("wav")
        || ct.contains("wave")
        || ct.contains("x-wav")
        || audio_bytes.starts_with(b"RIFF");

    if looks_wav {
        return extract_pcm_from_wav(audio_bytes);
    }

    // Already raw PCM (rare — only if a future path bypasses WAV wrapping).
    if ct.contains("pcm") || ct.contains("l16") || ct.contains("linear") {
        return Some(audio_bytes.to_vec());
    }

    None
}

/// Locates the `data` chunk of a RIFF/WAVE blob and returns its payload.
/// Tolerates extra chunks (fact, LIST, …) that sit between `fmt ` and `data`.
fn extract_pcm_from_wav(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        // Not a valid RIFF/WAVE — if the caller labelled it as wav but the
        // body is raw PCM after a fixed 44-byte header (our own encoder),
        // try the canonical offset as a last resort.
        if bytes.len() > 44 {
            return Some(bytes[44..].to_vec());
        }
        return None;
    }

    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size =
            u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        let data_start = offset + 8;
        let data_end = data_start.saturating_add(chunk_size);
        if data_end > bytes.len() {
            return None;
        }
        if chunk_id == b"data" {
            return Some(bytes[data_start..data_end].to_vec());
        }
        // Chunks are word-aligned; odd sizes are padded with one byte.
        offset = data_end + (chunk_size % 2);
    }

    // Fallback for our own 44-byte header encoder if chunk scan failed.
    if bytes.len() > 44 {
        Some(bytes[44..].to_vec())
    } else {
        None
    }
}
