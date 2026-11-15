//! Typed request/response contracts for Gemini audio services.

use serde::{Deserialize, Serialize};

use super::transport::GeminiAudioTransport;

/// Reference to a file already on the Gemini Files API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeminiFileRef {
    /// Resource name, e.g. `files/abc123`.
    pub name: String,
    /// URI passed to `file_data.file_uri`.
    pub uri: String,
    pub mime_type: String,
}

/// Which Gemini audio operation produced a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeminiOperation {
    Transcribe,
    Refine,
    Pronunciation,
}

/// Timing breakdown for a Gemini audio call (optional fields = not used).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeminiStageTiming {
    #[serde(default)]
    pub base64_ms: Option<u64>,
    #[serde(default)]
    pub files_upload_ms: Option<u64>,
    #[serde(default)]
    pub files_poll_ms: Option<u64>,
    #[serde(default)]
    pub files_poll_count: Option<u32>,
    #[serde(default)]
    pub generate_ms: Option<u64>,
    /// Only set when delete was awaited on the critical path (normally None).
    #[serde(default)]
    pub delete_ms: Option<u64>,
}

/// Successful generateContent outcome used by STT / refine paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiGenerateResult {
    pub operation: GeminiOperation,
    pub text: String,
    pub model: String,
    /// Prompt version marker for debugging / history later.
    pub prompt_version: String,
    #[serde(default)]
    pub latency_ms: u64,
    /// Remote file name if Files API was used.
    #[serde(default)]
    pub remote_file_name: Option<String>,
    #[serde(default)]
    pub transport: Option<GeminiAudioTransport>,
    #[serde(default)]
    pub timing: GeminiStageTiming,
    /// Provider-reported usage. Cost remains `unknown` unless the provider
    /// explicitly returns an actual charge.
    #[serde(default)]
    pub usage: crate::pipeline_run::UsageRecord,
}

/// Input for audio-only transcription.
#[derive(Debug, Clone)]
pub struct TranscribeRequest {
    pub audio_bytes: Vec<u8>,
    /// File extension (wav, mp3, …) used for MIME selection.
    pub ext: String,
    pub api_key: String,
    pub model: String,
    /// Optional display name for the Files API object.
    pub display_name: String,
    /// Audio duration when known (mic); used for transport selection.
    pub duration_ms: Option<u64>,
    /// Glossary block for FastAccurate (strict terms + aliases).
    pub glossary_block: String,
    /// Native custom vocabulary terms for dedicated speech models (acoustic biasing).
    pub custom_vocabulary: Vec<String>,
}
