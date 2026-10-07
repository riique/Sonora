//! Google Gemini (AI Studio) multimodal services.
//!
//! - [`pronunciation`] — oral proficiency evaluation (inline Base64)
//! - [`transcription`] — audio → text (inline or Files API)
//! - [`refinement`] — audio + draft → refined text
//! - [`files`] — Files API upload / poll / async delete
//! - [`transport`] — Inline vs Files API selection
//!
//! Direct Generative Language API only. No OpenRouter.

pub(crate) mod client;
mod files;
mod prompts;
mod pronunciation;
pub(crate) mod refinement;
mod transcription;
mod transport;
mod types;

#[cfg(test)]
mod mock;

pub use pronunciation::evaluate_pronunciation;
pub use refinement::{
    encode_audio_base64, refine_precise, refine_precise_with_file, refine_ultraprecise,
    refine_ultraprecise_with_file, refine_with_audio, transcribe_inline, transcribe_with_file,
};
pub use transcription::transcribe_audio;
pub use transport::{
    estimate_wav_duration_ms, select_gemini_audio_transport, GeminiAudioTransport,
    GEMINI_INLINE_MAX_BYTES, GEMINI_INLINE_MAX_DURATION_MS,
};
pub use types::{
    GeminiFileRef, GeminiGenerateResult, GeminiOperation, GeminiStageTiming, RefineRequest,
    TranscribeRequest,
};

pub(crate) use client::adaptive_generate_timeout;
pub use client::{is_transcribe_model, mime_for_ext};
pub use files::{spawn_cleanup, upload_and_wait, RemoteFileGuard, UploadTiming};
pub(crate) use prompts::{
    fast_accurate_transcription_prompt, precise_refinement_prompt, transcription_prompt,
    ultraprecise_refinement_prompt, GeminiPrompt, PRECISE_PROMPT_VERSION,
    TRANSCRIBE_PROMPT_VERSION, ULTRAPRECISE_PROMPT_VERSION,
};
