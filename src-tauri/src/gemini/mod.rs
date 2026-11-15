//! Google Gemini (AI Studio) multimodal services.
//!
//! - [`pronunciation`] — oral proficiency evaluation (inline Base64)
//! - [`transcription`] — audio → text (inline or Files API)
//! - [`files`] — Files API upload / poll / async delete
//! - [`transport`] — Inline vs Files API selection
//!
//! Direct Generative Language API only. No OpenRouter.

pub(crate) mod client;
mod files;
mod prompts;
mod pronunciation;
mod transcription;
mod transport;
mod types;

#[cfg(test)]
mod mock;

pub use pronunciation::evaluate_pronunciation;
pub use transcription::transcribe_audio;
pub use transport::{
    estimate_wav_duration_ms, select_gemini_audio_transport, GeminiAudioTransport,
    GEMINI_INLINE_MAX_BYTES, GEMINI_INLINE_MAX_DURATION_MS,
};
pub use types::{
    GeminiFileRef, GeminiGenerateResult, GeminiOperation, GeminiStageTiming, TranscribeRequest,
};

pub(crate) use client::adaptive_generate_timeout;
pub use client::{is_transcribe_model, mime_for_ext};
pub use files::{spawn_cleanup, upload_and_wait, RemoteFileGuard, UploadTiming};
pub(crate) use prompts::{
    fast_accurate_transcription_prompt, GeminiPrompt, TRANSCRIBE_PROMPT_VERSION,
};
