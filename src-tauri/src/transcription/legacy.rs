//! Stage 1 acoustic STT for the legacy engine / dual / Deepgram paths.

use std::sync::Arc;
use std::time::Duration;

use crate::models::{AppState, TranscriptionEngine};
use crate::transcription::fallback::{
    groq_err_to_message, resolve_dual_results, single_engine_slots,
};
use crate::transcription::types::AcousticOutcome;

/// Runs the configured Whisper fallback through OpenRouter, pinning Whisper
/// requests to Groq at the OpenRouter boundary. This keeps the fallback
/// provider observable and avoids bypassing the user's OpenRouter credential.
pub async fn transcribe_whisper_fallback_via_openrouter(
    state: &Arc<AppState>,
    bytes: &[u8],
    ext: &str,
) -> Result<(&'static str, crate::openrouter::OpenRouterGenerateResult), String> {
    let model = state
        .gemini_pipelines
        .read()
        .ultra_fast_whisper
        .openrouter_id();
    let api_key = state.next_openrouter_key().ok_or_else(|| {
        "Chave do OpenRouter não configurada para o fallback Whisper.".to_string()
    })?;
    log::info!(
        "transcription: dispatching Whisper fallback to OpenRouter model={} provider=groq",
        model
    );
    let vocabulary_hint = crate::vocabulary::whisper_prompt_hint(&state.vocabulary.read());
    let generated = crate::openrouter::transcribe_audio(
        bytes,
        ext,
        model,
        &api_key,
        Duration::from_secs(120),
        vocabulary_hint.as_deref(),
    )
    .await?;
    Ok((model, generated))
}

/// Routes `bytes` to the cloud STT provider selected by `engine`.
pub async fn transcribe_bytes(
    state: &Arc<AppState>,
    bytes: Vec<u8>,
    file_name: &str,
    mime: &str,
    engine: TranscriptionEngine,
) -> Result<String, String> {
    match engine {
        TranscriptionEngine::GroqWhisper => {
            let api_key = state.next_groq_key();
            let Some(api_key) = api_key else {
                log::error!("transcription: groq api key is missing, skipping transcription");
                return Err("Chave de API do Groq não configurada.".to_string());
            };
            log::info!("transcription: dispatching audio to groq whisper");
            match crate::groq::call_whisper_api(bytes, file_name, mime, &api_key).await {
                Ok(text) => {
                    log::info!(
                        "transcription: whisper transcription received ({} chars)",
                        text.len()
                    );
                    Ok(text)
                }
                Err(e) => {
                    log::error!("transcription: groq whisper transcription failed: {}", e);
                    Err(groq_err_to_message(&e))
                }
            }
        }
        TranscriptionEngine::DeepgramNova3 => {
            let api_key = state.next_deepgram_key();
            let Some(api_key) = api_key else {
                log::error!("transcription: deepgram api key is missing, skipping transcription");
                return Err("Chave de API do Deepgram não configurada.".to_string());
            };
            let mode = *state.deepgram_mode.read();
            log::info!(
                "transcription: dispatching audio to deepgram nova-3 (mode={})",
                mode.as_str()
            );
            let keyterms = {
                let vocab = state.vocabulary.read().clone();
                crate::vocabulary::deepgram_keyterms(&vocab, 20)
            };
            match crate::deepgram::transcribe_with_keyterms(bytes, mime, &api_key, mode, &keyterms)
                .await
            {
                Ok(text) => {
                    log::info!(
                        "transcription: deepgram transcription received ({} chars, mode={})",
                        text.len(),
                        mode.as_str()
                    );
                    Ok(text)
                }
                Err(e) => {
                    log::error!(
                        "transcription: deepgram transcription failed (mode={}): {}",
                        mode.as_str(),
                        e
                    );
                    Err(format!("Deepgram: {}", e))
                }
            }
        }
        other => {
            let msg = format!(
                "O motor {:?} não está conectado ao pipeline de captura de áudio.",
                other
            );
            log::error!("transcription: {}", msg);
            Err(msg)
        }
    }
}

/// Finishes a live Deepgram session if present; on failure falls back to batch REST.
pub async fn deepgram_from_live_or_posthoc(
    state: &Arc<AppState>,
    live: Option<crate::deepgram::DeepgramLiveSession>,
    wav: Vec<u8>,
    mime: &str,
) -> Result<String, String> {
    let api_key = state.next_deepgram_key();
    let Some(api_key) = api_key else {
        return Err("Chave de API do Deepgram não configurada.".to_string());
    };
    let mode = *state.deepgram_mode.read();

    if let Some(session) = live {
        let t0 = std::time::Instant::now();
        match session.finish().await {
            Ok(text) => {
                log::info!(
                    "transcription: deepgram LIVE finish in {}ms ({} chars) — no full re-upload",
                    t0.elapsed().as_millis(),
                    text.len()
                );
                return Ok(text);
            }
            Err(e) => {
                log::warn!(
                    "transcription: deepgram LIVE failed after stop ({}ms): {}; falling back to batch REST",
                    t0.elapsed().as_millis(),
                    e
                );
                return crate::deepgram::transcribe(
                    wav,
                    mime,
                    &api_key,
                    crate::models::DeepgramMode::Batch,
                )
                .await;
            }
        }
    }

    crate::deepgram::transcribe(wav, mime, &api_key, mode).await
}

/// Parallel Groq Whisper + Deepgram (post-hoc) for dual mode.
pub async fn run_dual_posthoc(
    state: &Arc<AppState>,
    bytes: Vec<u8>,
    file_name: &str,
    mime: &str,
) -> Result<AcousticOutcome, String> {
    let wav_clone = bytes.clone();
    let state1 = state.clone();
    let state2 = state.clone();
    let groq_fut = transcribe_bytes(
        &state1,
        wav_clone,
        file_name,
        mime,
        TranscriptionEngine::GroqWhisper,
    );
    let deepgram_fut = transcribe_bytes(
        &state2,
        bytes,
        file_name,
        mime,
        TranscriptionEngine::DeepgramNova3,
    );
    let (groq_res, deepgram_res) = tokio::join!(groq_fut, deepgram_fut);
    let (whisper_text, deepgram_text, effective_dual, deepgram_ran) =
        resolve_dual_results(groq_res, deepgram_res)?;
    Ok(AcousticOutcome {
        whisper_text,
        deepgram_text,
        effective_dual,
        deepgram_ran,
    })
}

/// Mic stop path: dual with optional live Deepgram, or single engine.
pub async fn run_acoustic_mic(
    state: &Arc<AppState>,
    wav: Vec<u8>,
    live_session: Option<crate::deepgram::DeepgramLiveSession>,
) -> Result<AcousticOutcome, String> {
    let dual_mode = *state.dual_engine.read();
    let engine = state.active_engine();
    log::info!(
        "transcription: active engine {:?} (dual_mode={}, live_deepgram={})",
        engine,
        dual_mode,
        live_session.is_some()
    );

    if dual_mode {
        let wav_clone = wav.clone();
        let state_clone1 = state.clone();
        let state_clone2 = state.clone();
        let groq_fut = transcribe_bytes(
            &state_clone1,
            wav_clone,
            "audio.wav",
            "audio/wav",
            TranscriptionEngine::GroqWhisper,
        );
        let deepgram_fut =
            deepgram_from_live_or_posthoc(&state_clone2, live_session, wav, "audio/wav");
        let (groq_res, deepgram_res) = tokio::join!(groq_fut, deepgram_fut);
        let (whisper_text, deepgram_text, effective_dual, deepgram_ran) =
            resolve_dual_results(groq_res, deepgram_res)?;
        return Ok(AcousticOutcome {
            whisper_text,
            deepgram_text,
            effective_dual,
            deepgram_ran,
        });
    }

    if engine == TranscriptionEngine::DeepgramNova3 {
        let text = deepgram_from_live_or_posthoc(state, live_session, wav, "audio/wav").await?;
        let (whisper_text, deepgram_text, deepgram_ran) =
            single_engine_slots(TranscriptionEngine::DeepgramNova3, text);
        return Ok(AcousticOutcome {
            whisper_text,
            deepgram_text,
            effective_dual: false,
            deepgram_ran,
        });
    }

    if let Some(session) = live_session {
        session.abort();
    }
    let text = transcribe_bytes(state, wav, "audio.wav", "audio/wav", engine).await?;
    let (whisper_text, deepgram_text, deepgram_ran) = single_engine_slots(engine, text);
    Ok(AcousticOutcome {
        whisper_text,
        deepgram_text,
        effective_dual: false,
        deepgram_ran,
    })
}

/// File upload / retry path (no live Deepgram session).
pub async fn run_acoustic_file(
    state: &Arc<AppState>,
    bytes: Vec<u8>,
    file_name: &str,
    mime: &str,
) -> Result<AcousticOutcome, String> {
    let dual_mode = *state.dual_engine.read();
    let engine = state.active_engine();

    if dual_mode {
        return run_dual_posthoc(state, bytes, file_name, mime).await;
    }

    let text = transcribe_bytes(state, bytes, file_name, mime, engine).await?;
    let (whisper_text, deepgram_text, deepgram_ran) = single_engine_slots(engine, text);
    Ok(AcousticOutcome {
        whisper_text,
        deepgram_text,
        effective_dual: false,
        deepgram_ran,
    })
}
