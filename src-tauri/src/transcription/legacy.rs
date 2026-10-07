//! Whisper helpers shared by the product modes.

use std::sync::Arc;
use std::time::Duration;

use crate::models::AppState;
use crate::transcription::fallback::groq_err_to_message;

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

/// Direct Groq Whisper call used by the Precise/UltraPrecise chains and the
/// manual "regenerate with fallback" action.
pub async fn transcribe_groq_whisper(
    state: &Arc<AppState>,
    bytes: Vec<u8>,
    file_name: &str,
    mime: &str,
) -> Result<String, String> {
    let Some(api_key) = state.next_groq_key() else {
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
