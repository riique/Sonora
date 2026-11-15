//! Whisper helpers shared by the product modes.

use std::sync::Arc;
use std::time::Duration;

use crate::models::AppState;

/// Whisper variant used as the automatic backup when the audio model fails.
pub const WHISPER_FALLBACK_MODEL: &str = "openai/whisper-large-v3-turbo";

/// Runs the configured Whisper fallback through OpenRouter, pinning Whisper
/// requests to Groq at the OpenRouter boundary. This keeps the fallback
/// provider observable and avoids bypassing the user's OpenRouter credential.
pub async fn transcribe_whisper_fallback_via_openrouter(
    state: &Arc<AppState>,
    bytes: &[u8],
    ext: &str,
) -> Result<(&'static str, crate::openrouter::OpenRouterGenerateResult), String> {
    let model = WHISPER_FALLBACK_MODEL;
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
