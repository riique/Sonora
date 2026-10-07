//! Stable product-mode identifiers and request compatibility helpers.
//!
//! [`PipelineRun`] is the single canonical execution result. The legacy
//! `PipelineResult` name remains only as a type alias for source compatibility.

use crate::models::{DeepgramMode, SanitizerModel, TranscriptionEngine};
pub use crate::pipeline_run::{
    PipelineRun, PipelineTimings, PipelineWarning, StageKind as PipelineStage, StageRecord,
};
pub type PipelineResult = PipelineRun;
use serde::{Deserialize, Deserializer, Serialize};

// ─── Modes & content ────────────────────────────────────────────────────────

/// Product-facing transcription mode (future selector).
///
/// Wire ids are stable kebab-case for settings.json / IPC.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TranscriptionMode {
    /// Whisper through OpenRouter's dedicated STT endpoint.
    #[default]
    UltraFast,
    /// Gemini with audio.
    FastAccurate,
    /// Whisper + Gemini with audio.
    Precise,
    /// Whisper + sanitizer + Gemini with audio.
    UltraPrecise,
}

impl<'de> Deserialize<'de> for TranscriptionMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "ultra-fast" => Ok(Self::UltraFast),
            "fast-accurate" | "chirp-3-experimental" => Ok(Self::FastAccurate),
            "precise" => Ok(Self::Precise),
            "ultra-precise" => Ok(Self::UltraPrecise),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &["ultra-fast", "fast-accurate", "precise", "ultra-precise"],
            )),
        }
    }
}

impl TranscriptionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UltraFast => "ultra-fast",
            Self::FastAccurate => "fast-accurate",
            Self::Precise => "precise",
            Self::UltraPrecise => "ultra-precise",
        }
    }

    /// Human label for future UI (clear product names).
    pub fn display_name_pt(self) -> &'static str {
        match self {
            Self::UltraFast => "Ultrarrápido",
            Self::FastAccurate => "Rápido e preciso",
            Self::Precise => "Preciso",
            Self::UltraPrecise => "Ultrapreciso",
        }
    }

    pub fn uses_whisper(self) -> bool {
        matches!(self, Self::UltraFast | Self::Precise | Self::UltraPrecise)
    }

    pub fn uses_gemini_audio(self) -> bool {
        matches!(
            self,
            Self::FastAccurate | Self::Precise | Self::UltraPrecise
        )
    }

    pub fn uses_sanitizer(self) -> bool {
        matches!(self, Self::UltraPrecise)
    }

    /// Maps current engine + dual flags onto the closest future mode.
    ///
    /// Runtime stays on the legacy path until a later phase adopts modes.
    /// Dual → Precise (multi-pass accuracy intent). Single acoustic → UltraFast.
    /// Gemini enum (unused for STT) → FastAccurate as the natural home.
    pub fn from_legacy(engine: TranscriptionEngine, dual_engine: bool) -> Self {
        if dual_engine {
            return Self::Precise;
        }
        match engine {
            TranscriptionEngine::GroqWhisper | TranscriptionEngine::DeepgramNova3 => {
                Self::UltraFast
            }
            TranscriptionEngine::GeminiMultimodal => Self::FastAccurate,
        }
    }
}

/// Optional content hint for future prompt routing (not applied yet).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContentType {
    #[default]
    Auto,
    Programming,
    Study,
}

impl<'de> Deserialize<'de> for ContentType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "auto" | "general-speech" => Ok(Self::Auto),
            "programming" => Ok(Self::Programming),
            "study" => Ok(Self::Study),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &["auto", "programming", "study"],
            )),
        }
    }
}

impl ContentType {
    pub fn parse_str(value: &str) -> Option<Self> {
        match value {
            "auto" | "general-speech" => Some(Self::Auto),
            "programming" => Some(Self::Programming),
            "study" => Some(Self::Study),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Programming => "programming",
            Self::Study => "study",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GeminiModel {
    #[default]
    FlashLite35,
    Flash36,
    #[serde(alias = "gemini-3.5-transcribe", alias = "transcribe-35")]
    Transcribe35,
    #[serde(alias = "muse-voice-transcribe-1.0", alias = "muse-voice-transcribe")]
    MuseVoiceTranscribe,
}

impl GeminiModel {
    pub fn google_id(self) -> &'static str {
        match self {
            Self::FlashLite35 => "gemini-3.5-flash-lite",
            Self::Flash36 => "gemini-3.6-flash",
            Self::Transcribe35 => "gemini-3.5-transcribe",
            Self::MuseVoiceTranscribe => "gemini-3.5-flash-lite",
        }
    }

    pub fn openrouter_id(self) -> &'static str {
        match self {
            Self::FlashLite35 => "google/gemini-3.5-flash-lite",
            Self::Flash36 => "google/gemini-3.6-flash",
            Self::Transcribe35 => "google/gemini-3.5-transcribe",
            Self::MuseVoiceTranscribe => "meta/muse-voice-transcribe-1.0",
        }
    }

    pub fn meta_id(self) -> &'static str {
        "muse-voice-transcribe-1.0"
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GeminiProvider {
    #[default]
    GoogleAiStudio,
    OpenRouter,
    Meta,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeminiPipelineChoice {
    #[serde(default)]
    pub model: GeminiModel,
    #[serde(default)]
    pub provider: GeminiProvider,
    #[serde(default)]
    pub use_custom_model: bool,
    /// When non-empty, replaces the selected preset with the provider model id
    /// exactly as entered in settings (apart from an optional models/ prefix,
    /// which is normalized by resolved_model_id).
    #[serde(default)]
    pub custom_model: String,
    #[serde(default = "default_meta_languages")]
    pub meta_languages: Vec<String>,
}

fn default_meta_languages() -> Vec<String> {
    vec!["Portuguese".to_string(), "English".to_string()]
}

impl GeminiPipelineChoice {
    pub fn resolved_model_id(&self) -> Result<String, String> {
        let custom = self.custom_model.trim();
        let model = if !self.use_custom_model {
            match self.provider {
                GeminiProvider::GoogleAiStudio => self.model.google_id().to_string(),
                GeminiProvider::OpenRouter => self.model.openrouter_id().to_string(),
                GeminiProvider::Meta => self.model.meta_id().to_string(),
            }
        } else {
            custom.strip_prefix("models/").unwrap_or(custom).to_string()
        };

        if model.is_empty()
            || model.chars().any(char::is_whitespace)
            || model.contains('?')
            || model.contains('#')
        {
            return Err("Informe um ID de modelo válido, sem espaços, '?' ou '#'.".to_string());
        }
        Ok(model)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UltraFastProvider {
    #[default]
    OpenRouter,
    Groq,
}

impl UltraFastProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenRouter => "open-router",
            Self::Groq => "groq",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenRouterWhisperModel {
    #[default]
    LargeV3Turbo,
    LargeV3,
}

impl OpenRouterWhisperModel {
    pub fn openrouter_id(self) -> &'static str {
        match self {
            Self::LargeV3Turbo => "openai/whisper-large-v3-turbo",
            Self::LargeV3 => "openai/whisper-large-v3",
        }
    }

    /// Groq direct model id for the same Whisper variant.
    /// Both ids are served by the Groq Audio Transcriptions endpoint.
    pub fn groq_id(self) -> &'static str {
        match self {
            Self::LargeV3Turbo => "whisper-large-v3-turbo",
            Self::LargeV3 => "whisper-large-v3",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeminiPipelineConfig {
    /// Transport for the UltraFast pipeline: OpenRouter STT (pinned to Groq)
    /// or Groq Audio Transcriptions directly. Old installs without this field
    /// default to OpenRouter, preserving current behavior.
    #[serde(default)]
    pub ultra_fast_provider: UltraFastProvider,
    #[serde(default)]
    pub ultra_fast_whisper: OpenRouterWhisperModel,
    #[serde(default)]
    pub fast_accurate: GeminiPipelineChoice,
    #[serde(default)]
    pub precise: GeminiPipelineChoice,
    #[serde(default)]
    pub ultra_precise: GeminiPipelineChoice,
}

// ─── Stage identity ─────────────────────────────────────────────────────────

// ─── Request / result contracts ─────────────────────────────────────────────

/// Input to a future pipeline run. Carries audio + resolved preferences.
/// Not consumed by `audio.rs` in phase 01.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRequest {
    /// Stable id (history / audio file stem).
    pub id: String,
    /// WAV or original upload bytes (not persisted inside this struct on disk).
    #[serde(skip)]
    pub audio_bytes: Vec<u8>,
    pub file_name: String,
    pub mime: String,
    pub mode: TranscriptionMode,
    #[serde(default)]
    pub content_type: ContentType,
    /// Legacy engine still selected in settings (preserved for dual/Deepgram).
    pub engine: TranscriptionEngine,
    pub dual_engine: bool,
    pub deepgram_mode: DeepgramMode,
    pub sanitizer: SanitizerModel,
    pub sanitizer_enabled: bool,
    pub reasoning_enabled: bool,
    pub reasoning_effort: String,
    pub system_prompt: String,
    #[serde(default)]
    pub custom_words: Vec<String>,
    /// `"mic"` | `"file"`.
    pub source: String,
    pub duration_ms: u64,
    pub copy_to_clipboard: bool,
}

pub struct FromLegacySettingsInput {
    pub id: String,
    pub audio_bytes: Vec<u8>,
    pub file_name: String,
    pub mime: String,
    pub engine: TranscriptionEngine,
    pub dual_engine: bool,
    pub deepgram_mode: DeepgramMode,
    pub sanitizer: SanitizerModel,
    pub sanitizer_enabled: bool,
    pub reasoning_enabled: bool,
    pub reasoning_effort: String,
    pub system_prompt: String,
    pub custom_words: Vec<String>,
    pub source: String,
    pub duration_ms: u64,
    pub copy_to_clipboard: bool,
}

impl PipelineRequest {
    /// Builds a request snapshot from the **current** legacy settings shape
    /// without dropping any preference the user already has.
    pub fn from_legacy_settings(input: FromLegacySettingsInput) -> Self {
        let FromLegacySettingsInput {
            id,
            audio_bytes,
            file_name,
            mime,
            engine,
            dual_engine,
            deepgram_mode,
            sanitizer,
            sanitizer_enabled,
            reasoning_enabled,
            reasoning_effort,
            system_prompt,
            custom_words,
            source,
            duration_ms,
            copy_to_clipboard,
        } = input;
        let engine_v = engine;
        let dual_v = dual_engine;
        Self {
            id,
            audio_bytes,
            file_name,
            mime,
            mode: TranscriptionMode::from_legacy(engine_v, dual_v),
            content_type: ContentType::default(),
            engine: engine_v,
            dual_engine: dual_v,
            deepgram_mode,
            sanitizer,
            sanitizer_enabled,
            reasoning_enabled,
            reasoning_effort,
            system_prompt,
            custom_words,
            source,
            duration_ms,
            copy_to_clipboard,
        }
    }
}

// ─── Pure helpers (testable without network) ────────────────────────────────

/// Sentinel the live sanitizer may return (mirrors `groq::FALLBACK_RETRY_SENTINEL`).
pub const FALLBACK_RETRY_SENTINEL: &str = "[FALLBACK_RETRY]";

/// Strips common LLM preambles / headers the sanitizer is instructed not to emit.
/// Used by future finalize logic and covered by unit tests now.
pub fn strip_sanitizer_artifacts(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut lines: Vec<&str> = trimmed.lines().collect();
    // Drop leading markdown headings / labels that models sometimes leak.
    while let Some(first) = lines.first() {
        let t = first.trim();
        let lower = t.to_ascii_lowercase();
        let is_header = t.starts_with('#')
            || lower.starts_with("here is")
            || lower.starts_with("here's")
            || lower.starts_with("aqui está")
            || lower.starts_with("aqui esta")
            || lower.starts_with("texto final")
            || lower.starts_with("transcrição")
            || lower.starts_with("transcricao")
            || lower.starts_with("cleaned:")
            || lower.starts_with("output:");
        if is_header && lines.len() > 1 {
            lines.remove(0);
        } else {
            break;
        }
    }

    // Drop trailing glossary / notes blocks.
    if let Some(idx) = lines.iter().position(|l| {
        let lower = l.trim().to_ascii_lowercase();
        lower.starts_with("glossário")
            || lower.starts_with("glossario")
            || lower.starts_with("nota:")
            || lower.starts_with("notes:")
            || lower.starts_with("---")
    }) {
        if idx > 0 {
            lines.truncate(idx);
        }
    }

    lines
        .join("\n")
        .trim()
        .trim_matches(|c| c == '"' || c == '\u{201c}' || c == '\u{201d}')
        .to_string()
}

/// Picks the best raw acoustic when sanitizer is off or failed (mirrors
/// `audio::pick_raw_acoustic` policy for contract tests).
pub fn pick_raw_acoustic(whisper_text: &str, deepgram_text: &str) -> String {
    let w = whisper_text.trim();
    let d = deepgram_text.trim();
    match (w.is_empty(), d.is_empty()) {
        (true, true) => String::new(),
        (false, true) => w.to_string(),
        (true, false) => d.to_string(),
        (false, false) => {
            if w.eq_ignore_ascii_case(d) {
                return w.to_string();
            }
            let wl = w.chars().count();
            let dl = d.chars().count();
            if wl > dl.saturating_add(8) {
                w.to_string()
            } else {
                d.to_string()
            }
        }
    }
}

/// When finalized text is blank, prefer Deepgram raw then Whisper raw.
pub fn coalesce_empty_final(
    finalized: String,
    whisper_text: &str,
    deepgram_text: &str,
) -> (String, bool, Option<String>) {
    if !finalized.trim().is_empty() {
        return (finalized, false, None);
    }
    if !deepgram_text.trim().is_empty() {
        return (
            deepgram_text.trim().to_string(),
            true,
            Some("sanitizer_empty_prefer_deepgram".into()),
        );
    }
    if !whisper_text.trim().is_empty() {
        return (
            whisper_text.trim().to_string(),
            true,
            Some("sanitizer_empty_prefer_whisper".into()),
        );
    }
    (finalized, false, None)
}

/// Resolves sanitizer output into deliverable text + fallback metadata.
pub fn resolve_sanitizer_output(
    sanitizer_raw: Option<&str>,
    whisper_text: &str,
    deepgram_text: &str,
) -> (String, bool, Option<String>, Vec<PipelineWarning>) {
    let mut warnings = Vec::new();
    let Some(raw) = sanitizer_raw else {
        let picked = pick_raw_acoustic(whisper_text, deepgram_text);
        warnings.push(PipelineWarning::new(
            PipelineStage::Sanitizer,
            "sanitizer_skipped_or_failed",
            "Validador indisponível; usando texto acústico.",
        ));
        return (picked, true, Some("sanitizer_missing".into()), warnings);
    };

    let cleaned = strip_sanitizer_artifacts(raw);
    if cleaned.trim() == FALLBACK_RETRY_SENTINEL {
        let picked = pick_raw_acoustic(whisper_text, deepgram_text);
        warnings.push(PipelineWarning::new(
            PipelineStage::Sanitizer,
            "fallback_retry_sentinel",
            "Validador pediu fallback para o texto acústico.",
        ));
        return (
            picked,
            true,
            Some("fallback_retry_sentinel".into()),
            warnings,
        );
    }
    if cleaned.trim().is_empty() {
        let (text, used, reason) = coalesce_empty_final(String::new(), whisper_text, deepgram_text);
        warnings.push(PipelineWarning::new(
            PipelineStage::Sanitizer,
            "sanitizer_empty",
            "Validador devolveu texto vazio.",
        ));
        return (text, used, reason, warnings);
    }

    // Header-only cleanup already applied; if strip removed everything meaningful
    // but original had content that was only headers, fall back.
    if cleaned.trim().is_empty() && !raw.trim().is_empty() {
        let picked = pick_raw_acoustic(whisper_text, deepgram_text);
        warnings.push(PipelineWarning::new(
            PipelineStage::Sanitizer,
            "sanitizer_header_only",
            "Resposta do validador continha só cabeçalhos.",
        ));
        return (picked, true, Some("sanitizer_header_only".into()), warnings);
    }

    (cleaned, false, None, warnings)
}

/// Dual-engine partial failure: keep the surviving acoustic stream.
pub fn resolve_dual_partial(
    whisper: Result<String, String>,
    deepgram: Result<String, String>,
) -> (
    String,
    String,
    bool,
    bool,
    Vec<PipelineWarning>,
    Option<String>,
) {
    let mut warnings = Vec::new();
    match (whisper, deepgram) {
        (Ok(w), Ok(d)) => (w, d, true, true, warnings, None),
        (Ok(w), Err(e)) => {
            warnings.push(PipelineWarning::new(
                PipelineStage::Deepgram,
                "dual_deepgram_failed",
                format!("Deepgram falhou no modo duplo: {e}"),
            ));
            (w, String::new(), false, false, warnings, None)
        }
        (Err(e), Ok(d)) => {
            warnings.push(PipelineWarning::new(
                PipelineStage::Whisper,
                "dual_whisper_failed",
                format!("Whisper falhou no modo duplo: {e}"),
            ));
            (String::new(), d, false, true, warnings, None)
        }
        (Err(we), Err(de)) => {
            let msg = format!("Ambos os motores falharam:\n• Whisper: {we}\n• Deepgram: {de}");
            warnings.push(PipelineWarning::new(
                PipelineStage::Finalize,
                "dual_both_failed",
                msg.clone(),
            ));
            (
                String::new(),
                String::new(),
                false,
                false,
                warnings,
                Some(msg),
            )
        }
    }
}

// ─── Legacy settings snapshot (compat, no drop) ─────────────────────────────

/// On-disk-shaped preferences that must round-trip when new mode fields appear.
/// Mirrors the fields users already have; optional mode/content default safely.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LegacySettingsSnapshot {
    #[serde(default)]
    pub engine: Option<TranscriptionEngine>,
    #[serde(default)]
    pub sanitizer: Option<SanitizerModel>,
    #[serde(default)]
    pub dual_engine: bool,
    #[serde(default)]
    pub deepgram_mode: DeepgramMode,
    #[serde(default = "default_true")]
    pub sanitizer_enabled: bool,
    #[serde(default)]
    pub reasoning_enabled: bool,
    #[serde(default = "default_effort")]
    pub reasoning_effort: String,
    #[serde(default)]
    pub custom_words: Vec<String>,
    /// Future field: absent on old installs → default UltraFast mapping via from_legacy.
    #[serde(default)]
    pub transcription_mode: Option<TranscriptionMode>,
    #[serde(default)]
    pub content_type: ContentType,
}

fn default_true() -> bool {
    true
}

fn default_effort() -> String {
    "medium".to_string()
}

impl Default for LegacySettingsSnapshot {
    fn default() -> Self {
        Self {
            engine: Some(TranscriptionEngine::GroqWhisper),
            sanitizer: Some(SanitizerModel::Llama70b),
            dual_engine: false,
            deepgram_mode: DeepgramMode::Batch,
            sanitizer_enabled: true,
            reasoning_enabled: false,
            reasoning_effort: "medium".into(),
            custom_words: Vec::new(),
            transcription_mode: None,
            content_type: ContentType::Auto,
        }
    }
}

impl LegacySettingsSnapshot {
    /// Effective mode without discarding legacy engine/dual.
    pub fn effective_mode(&self) -> TranscriptionMode {
        self.transcription_mode.unwrap_or_else(|| {
            TranscriptionMode::from_legacy(self.engine.unwrap_or_default(), self.dual_engine)
        })
    }

    /// Ensures serialize → deserialize keeps every legacy field the user had.
    pub fn round_trip_json(&self) -> Result<Self, String> {
        let s = serde_json::to_string(self).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| e.to_string())
    }
}

// ─── Local mocks (no network) ───────────────────────────────────────────────

/// In-memory fake providers for unit tests.
#[derive(Debug, Clone, Default)]
pub struct MockProviders {
    pub whisper: Option<Result<String, String>>,
    pub deepgram: Option<Result<String, String>>,
    pub sanitizer: Option<Result<String, String>>,
    pub gemini: Option<Result<String, String>>,
}

fn mock_stage(
    stage: PipelineStage,
    ok: bool,
    text: Option<String>,
    model: Option<String>,
    latency_ms: u64,
    error: Option<String>,
) -> StageRecord {
    let mut record = StageRecord::completed(stage, latency_ms);
    record.model = model;
    record.status = if ok {
        crate::pipeline_run::StageStatus::Success
    } else {
        crate::pipeline_run::StageStatus::Failed
    };
    if let Some(text) = text {
        record.metadata.insert(
            "output_chars".into(),
            serde_json::Value::from(text.chars().count() as u64),
        );
    }
    record.error = error.map(|message| crate::pipeline_run::PipelineError {
        kind: crate::pipeline_run::PipelineErrorKind::Provider,
        code: "mock_provider_failed".into(),
        message,
        retryable: true,
    });
    record
}

/// Runs a **local** mock pipeline for the given mode. Never touches the network.
pub fn run_mock_pipeline(req: &PipelineRequest, mocks: &MockProviders) -> PipelineResult {
    let mut result = PipelineResult::success(&req.id, req.mode, "");
    result.content_hint = req.content_type;
    result.history_engine_label = match (req.dual_engine, req.engine) {
        (true, _) => "Groq+Deepgram".into(),
        (false, TranscriptionEngine::DeepgramNova3) => "DeepgramNova3".into(),
        (false, TranscriptionEngine::GroqWhisper) => "GroqWhisper".into(),
        (false, TranscriptionEngine::GeminiMultimodal) => "GeminiMultimodal".into(),
    };

    let mut whisper_text = String::new();
    let mut deepgram_text = String::new();

    // Acoustic stages depending on mode + legacy dual.
    let want_whisper = req.mode.uses_whisper()
        || req.dual_engine
        || req.engine == TranscriptionEngine::GroqWhisper;
    let want_deepgram = req.dual_engine || req.engine == TranscriptionEngine::DeepgramNova3;
    // Future modes that ignore Deepgram still allow legacy dual path in mock
    // when dual_engine is true so we never "lose" dual in tests.

    if want_whisper {
        match mocks.whisper.clone().unwrap_or_else(|| Ok(String::new())) {
            Ok(t) => {
                whisper_text = t.clone();
                result.whisper_text = Some(t.clone());
                result.journal.push(mock_stage(
                    PipelineStage::Whisper,
                    true,
                    Some(t),
                    Some("whisper-large-v3-turbo".into()),
                    10,
                    None,
                ));
                result.models_used.push("whisper-large-v3-turbo".into());
                result.timings.whisper_ms = Some(10);
            }
            Err(e) => {
                result.journal.push(mock_stage(
                    PipelineStage::Whisper,
                    false,
                    None,
                    Some("whisper-large-v3-turbo".into()),
                    10,
                    Some(e.clone()),
                ));
                if !want_deepgram && !req.mode.uses_gemini_audio() {
                    result.is_error = true;
                    result.error_message = Some(e);
                    result.timings.recompute_total();
                    return result;
                }
            }
        }
    }

    if want_deepgram {
        match mocks.deepgram.clone().unwrap_or_else(|| Ok(String::new())) {
            Ok(t) => {
                deepgram_text = t.clone();
                result.deepgram_text = Some(t.clone());
                result.journal.push(mock_stage(
                    PipelineStage::Deepgram,
                    true,
                    Some(t),
                    Some("nova-3".into()),
                    12,
                    None,
                ));
                result.models_used.push("nova-3".into());
                result.timings.deepgram_ms = Some(12);
            }
            Err(e) => {
                result.journal.push(mock_stage(
                    PipelineStage::Deepgram,
                    false,
                    None,
                    Some("nova-3".into()),
                    12,
                    Some(e.clone()),
                ));
                if whisper_text.is_empty() && !req.mode.uses_gemini_audio() {
                    result.is_error = true;
                    result.error_message = Some(e);
                    result.timings.recompute_total();
                    return result;
                }
                result.journal_warnings.push(PipelineWarning::new(
                    PipelineStage::Deepgram,
                    "deepgram_failed",
                    e,
                ));
            }
        }
    }

    // Dual partial resolution when both were requested.
    if req.dual_engine {
        let w = if result.journal.iter().any(|s| {
            s.stage == PipelineStage::Whisper
                && s.status == crate::pipeline_run::StageStatus::Success
        }) {
            Ok(whisper_text.clone())
        } else if result.journal.iter().any(|s| {
            s.stage == PipelineStage::Whisper
                && s.status == crate::pipeline_run::StageStatus::Failed
        }) {
            Err(result
                .journal
                .iter()
                .find(|s| s.stage == PipelineStage::Whisper)
                .and_then(|s| s.error.as_ref().map(|error| error.message.clone()))
                .unwrap_or_else(|| "whisper failed".into()))
        } else {
            Ok(String::new())
        };
        let d = if result.journal.iter().any(|s| {
            s.stage == PipelineStage::Deepgram
                && s.status == crate::pipeline_run::StageStatus::Success
        }) {
            Ok(deepgram_text.clone())
        } else if result.journal.iter().any(|s| {
            s.stage == PipelineStage::Deepgram
                && s.status == crate::pipeline_run::StageStatus::Failed
        }) {
            Err(result
                .journal
                .iter()
                .find(|s| s.stage == PipelineStage::Deepgram)
                .and_then(|s| s.error.as_ref().map(|error| error.message.clone()))
                .unwrap_or_else(|| "deepgram failed".into()))
        } else {
            Ok(String::new())
        };
        let (w2, d2, _eff, _dg, warns, hard) = resolve_dual_partial(w, d);
        whisper_text = w2;
        deepgram_text = d2;
        result.journal_warnings.extend(warns);
        if let Some(msg) = hard {
            result.is_error = true;
            result.error_message = Some(msg);
            result.timings.recompute_total();
            return result;
        }
    }

    if whisper_text.trim().is_empty()
        && deepgram_text.trim().is_empty()
        && !req.mode.uses_gemini_audio()
    {
        result.is_error = true;
        result.error_message = Some("Nenhum texto detectado na gravação.".into());
        result.timings.recompute_total();
        return result;
    }

    // Sanitizer: UltraPrecise always; UltraFast when legacy sanitizer_enabled.
    // FastAccurate / Precise skip sanitizer in the mock (Gemini path owns refine).
    let run_sanitizer = match req.mode {
        TranscriptionMode::UltraPrecise => true,
        TranscriptionMode::UltraFast => req.sanitizer_enabled,
        TranscriptionMode::FastAccurate | TranscriptionMode::Precise => false,
    };

    if run_sanitizer {
        let san_res = mocks
            .sanitizer
            .clone()
            .unwrap_or_else(|| Ok(pick_raw_acoustic(&whisper_text, &deepgram_text)));
        match san_res {
            Ok(raw) => {
                let (text, used_fb, reason, warns) =
                    resolve_sanitizer_output(Some(&raw), &whisper_text, &deepgram_text);
                result.sanitizer_text = Some(raw);
                result.journal_warnings.extend(warns);
                result.used_fallback = used_fb;
                result.fallback_reason = reason;
                result.final_text = text;
                result.journal.push(mock_stage(
                    PipelineStage::Sanitizer,
                    true,
                    Some(result.final_text.clone()),
                    Some(req.sanitizer.api_model_id().into()),
                    20,
                    None,
                ));
                result.models_used.push(req.sanitizer.api_model_id().into());
                result.timings.sanitizer_ms = Some(20);
            }
            Err(e) => {
                let (text, _, reason, warns) =
                    resolve_sanitizer_output(None, &whisper_text, &deepgram_text);
                result.journal_warnings.extend(warns);
                result.journal_warnings.push(PipelineWarning::new(
                    PipelineStage::Sanitizer,
                    "sanitizer_error",
                    e,
                ));
                result.used_fallback = true;
                result.fallback_reason = reason.or(Some("sanitizer_error".into()));
                result.final_text = text;
                result.journal.push(mock_stage(
                    PipelineStage::Sanitizer,
                    false,
                    Some(result.final_text.clone()),
                    Some(req.sanitizer.api_model_id().into()),
                    20,
                    result.fallback_reason.clone(),
                ));
                result.timings.sanitizer_ms = Some(20);
            }
        }
    } else {
        result.final_text = pick_raw_acoustic(&whisper_text, &deepgram_text);
    }

    // Gemini audio stage (future modes).
    if req.mode.uses_gemini_audio() {
        match mocks.gemini.clone().unwrap_or_else(|| Ok(String::new())) {
            Ok(t) if !t.trim().is_empty() => {
                result.gemini_text = Some(t.clone());
                // Precise / UltraPrecise: Gemini refines; FastAccurate: Gemini is source.
                if result.final_text.trim().is_empty()
                    || req.mode == TranscriptionMode::FastAccurate
                {
                    result.final_text = t.clone();
                } else if req.mode == TranscriptionMode::Precise
                    || req.mode == TranscriptionMode::UltraPrecise
                {
                    // Prefer non-empty Gemini refine when provided by mock.
                    result.final_text = t.clone();
                }
                result.journal.push(mock_stage(
                    PipelineStage::GeminiAudio,
                    true,
                    Some(t),
                    Some("gemini-3.5-flash-lite".into()),
                    40,
                    None,
                ));
                result.models_used.push("gemini-3.5-flash-lite".into());
                result.timings.gemini_ms = Some(40);
            }
            Ok(_) => {
                result.journal_warnings.push(PipelineWarning::new(
                    PipelineStage::GeminiAudio,
                    "gemini_empty",
                    "Gemini devolveu texto vazio; mantendo etapa anterior.",
                ));
                result.used_fallback = true;
                result.fallback_reason = Some("gemini_empty".into());
                if result.final_text.trim().is_empty() {
                    result.final_text = pick_raw_acoustic(&whisper_text, &deepgram_text);
                }
            }
            Err(e) => {
                result.journal_warnings.push(PipelineWarning::new(
                    PipelineStage::GeminiAudio,
                    "gemini_failed",
                    e,
                ));
                result.used_fallback = true;
                result.fallback_reason = Some("gemini_failed".into());
                if result.final_text.trim().is_empty() {
                    result.final_text = pick_raw_acoustic(&whisper_text, &deepgram_text);
                }
                if result.final_text.trim().is_empty() {
                    result.is_error = true;
                    result.error_message =
                        Some("Falha no Gemini e sem texto acústico de reserva.".into());
                }
            }
        }
    }

    if result.final_text.trim().is_empty() && !result.is_error {
        result.is_error = true;
        result.error_message = Some("Nenhum texto detectado na gravação.".into());
    }

    result.timings.recompute_total();
    result.normalize();
    result
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TranscriptionEngine;

    #[test]
    fn mode_serde_roundtrip_and_defaults() {
        assert_eq!(TranscriptionMode::default(), TranscriptionMode::UltraFast);
        for mode in [
            TranscriptionMode::UltraFast,
            TranscriptionMode::FastAccurate,
            TranscriptionMode::Precise,
            TranscriptionMode::UltraPrecise,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            let back: TranscriptionMode = serde_json::from_str(&json).unwrap();
            assert_eq!(back, mode);
            assert_eq!(
                serde_json::from_str::<TranscriptionMode>(&format!("\"{}\"", mode.as_str()))
                    .unwrap(),
                mode
            );
        }
    }

    #[test]
    fn content_type_serde_and_default() {
        assert_eq!(ContentType::default(), ContentType::Auto);
        let json = r#""programming""#;
        assert_eq!(
            serde_json::from_str::<ContentType>(json).unwrap(),
            ContentType::Programming
        );
        for ct in [
            ContentType::Auto,
            ContentType::Programming,
            ContentType::Study,
        ] {
            let s = serde_json::to_string(&ct).unwrap();
            let back: ContentType = serde_json::from_str(&s).unwrap();
            assert_eq!(back, ct);
        }
    }

    #[test]
    fn removed_options_migrate_safely() {
        assert_eq!(
            serde_json::from_str::<TranscriptionMode>(r#""chirp-3-experimental""#).unwrap(),
            TranscriptionMode::FastAccurate
        );
        assert_eq!(
            serde_json::from_str::<ContentType>(r#""general-speech""#).unwrap(),
            ContentType::Auto
        );
    }

    #[test]
    fn unknown_mode_json_fails_closed() {
        let err = serde_json::from_str::<TranscriptionMode>(r#""teleport""#);
        assert!(err.is_err());
    }

    #[test]
    fn unknown_content_type_fails_closed() {
        let err = serde_json::from_str::<ContentType>(r#""podcast""#);
        assert!(err.is_err());
    }

    #[test]
    fn legacy_settings_without_mode_field_still_load() {
        let old = r#"{
            "engine": "groq-whisper",
            "sanitizer": "llama-70b",
            "dual_engine": true,
            "deepgram_mode": "streaming_final",
            "sanitizer_enabled": true,
            "reasoning_enabled": false,
            "reasoning_effort": "medium",
            "custom_words": ["Sonora", "Tokio"]
        }"#;
        let snap: LegacySettingsSnapshot = serde_json::from_str(old).unwrap();
        assert_eq!(snap.engine, Some(TranscriptionEngine::GroqWhisper));
        assert_eq!(snap.sanitizer, Some(SanitizerModel::Llama70b));
        assert!(snap.dual_engine);
        assert_eq!(snap.deepgram_mode, DeepgramMode::StreamingFinal);
        assert_eq!(snap.custom_words, vec!["Sonora", "Tokio"]);
        assert!(snap.transcription_mode.is_none());
        assert_eq!(snap.effective_mode(), TranscriptionMode::Precise);
        // Round-trip keeps vocabulary and dual.
        let back = snap.round_trip_json().unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn legacy_settings_empty_object_defaults_safe() {
        let snap: LegacySettingsSnapshot = serde_json::from_str("{}").unwrap();
        assert!(!snap.dual_engine);
        assert!(snap.sanitizer_enabled);
        assert_eq!(snap.deepgram_mode, DeepgramMode::Batch);
        assert_eq!(snap.reasoning_effort, "medium");
        assert_eq!(snap.content_type, ContentType::Auto);
    }

    #[test]
    fn from_legacy_mapping_preserves_intent() {
        assert_eq!(
            TranscriptionMode::from_legacy(TranscriptionEngine::GroqWhisper, false),
            TranscriptionMode::UltraFast
        );
        assert_eq!(
            TranscriptionMode::from_legacy(TranscriptionEngine::DeepgramNova3, false),
            TranscriptionMode::UltraFast
        );
        assert_eq!(
            TranscriptionMode::from_legacy(TranscriptionEngine::GroqWhisper, true),
            TranscriptionMode::Precise
        );
        assert_eq!(
            TranscriptionMode::from_legacy(TranscriptionEngine::GeminiMultimodal, false),
            TranscriptionMode::FastAccurate
        );
    }

    #[test]
    fn pipeline_request_from_legacy_keeps_keys_of_config() {
        let req = PipelineRequest::from_legacy_settings(
            crate::pipeline_contract::FromLegacySettingsInput {
                id: ("1").into(),
                audio_bytes: vec![1, 2, 3],
                file_name: ("audio.wav").into(),
                mime: ("audio/wav").into(),
                engine: TranscriptionEngine::DeepgramNova3,
                dual_engine: true,
                deepgram_mode: DeepgramMode::StreamingFinal,
                sanitizer: SanitizerModel::GptOss120b,
                sanitizer_enabled: true,
                reasoning_enabled: true,
                reasoning_effort: ("high").into(),
                system_prompt: ("system").into(),
                custom_words: vec!["Sonora".into()],
                source: ("mic").into(),
                duration_ms: 1500,
                copy_to_clipboard: true,
            },
        );
        assert_eq!(req.engine, TranscriptionEngine::DeepgramNova3);
        assert!(req.dual_engine);
        assert_eq!(req.deepgram_mode, DeepgramMode::StreamingFinal);
        assert_eq!(req.sanitizer, SanitizerModel::GptOss120b);
        assert_eq!(req.reasoning_effort, "high");
        assert_eq!(req.custom_words, vec!["Sonora"]);
        assert_eq!(req.mode, TranscriptionMode::Precise);
        // audio_bytes skipped in serde but present in memory
        assert_eq!(req.audio_bytes.len(), 3);
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("audio_bytes"));
    }

    #[test]
    fn pipeline_result_serde_roundtrip() {
        let mut r = PipelineResult::success("42", TranscriptionMode::UltraPrecise, "olá mundo");
        r.whisper_text = Some("ola mundo".into());
        r.journal_warnings
            .push(PipelineWarning::new(PipelineStage::Sanitizer, "x", "y"));
        r.timings.whisper_ms = Some(11);
        r.timings.sanitizer_ms = Some(22);
        r.timings.recompute_total();
        r.models_used = vec![
            "whisper-large-v3-turbo".into(),
            "openai/gpt-oss-120b".into(),
        ];
        r.normalize();
        let json = serde_json::to_string_pretty(&r).unwrap();
        let back: PipelineResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.canonical_text(), "olá mundo");
        assert_eq!(back.timings.total_ms, 33);
        assert_eq!(back.journal_warnings.len(), 1);
        assert_eq!(back.mode, TranscriptionMode::UltraPrecise);
    }

    #[test]
    fn strip_sanitizer_header_and_glossary() {
        let raw = "# Texto final\nHere is the cleaned text:\nOlá, Sonora.\n\nGlossário:\n- foo";
        let cleaned = strip_sanitizer_artifacts(raw);
        assert!(cleaned.contains("Olá, Sonora"));
        assert!(!cleaned.to_ascii_lowercase().contains("gloss"));
        assert!(!cleaned.contains("Here is"));
    }

    #[test]
    fn sanitizer_empty_falls_back_to_deepgram() {
        let (text, used, reason, warns) =
            resolve_sanitizer_output(Some("   "), "whisper only", "deepgram wins");
        assert_eq!(text, "deepgram wins");
        assert!(used);
        assert_eq!(reason.as_deref(), Some("sanitizer_empty_prefer_deepgram"));
        assert!(!warns.is_empty());
    }

    #[test]
    fn sanitizer_sentinel_falls_back_to_raw() {
        let (text, used, reason, _) =
            resolve_sanitizer_output(Some(FALLBACK_RETRY_SENTINEL), "w text", "");
        assert_eq!(text, "w text");
        assert!(used);
        assert_eq!(reason.as_deref(), Some("fallback_retry_sentinel"));
    }

    #[test]
    fn sanitizer_missing_uses_pick_raw() {
        let (text, used, _, _) =
            resolve_sanitizer_output(None, "short", "much longer deepgram text here");
        assert_eq!(text, "much longer deepgram text here");
        assert!(used);
    }

    #[test]
    fn empty_stt_both_sides() {
        let (w, d, _, _, _, hard) = resolve_dual_partial(Ok(String::new()), Ok(String::new()));
        assert!(w.is_empty() && d.is_empty());
        assert!(hard.is_none());
        let (text, _, _) = coalesce_empty_final(String::new(), "", "");
        assert!(text.is_empty());
    }

    #[test]
    fn dual_partial_whisper_ok_deepgram_fail() {
        let (w, d, eff, dg, warns, hard) =
            resolve_dual_partial(Ok("hello".into()), Err("401".into()));
        assert_eq!(w, "hello");
        assert!(d.is_empty());
        assert!(!eff && !dg);
        assert!(hard.is_none());
        assert_eq!(warns[0].code, "dual_deepgram_failed");
    }

    #[test]
    fn dual_both_fail_hard_error() {
        let (_, _, _, _, _, hard) = resolve_dual_partial(Err("down".into()), Err("down2".into()));
        assert!(hard.unwrap().contains("Whisper"));
    }

    #[test]
    fn mock_ultra_fast_whisper_only() {
        let req = PipelineRequest::from_legacy_settings(
            crate::pipeline_contract::FromLegacySettingsInput {
                id: ("m1").into(),
                audio_bytes: vec![],
                file_name: ("a.wav").into(),
                mime: ("audio/wav").into(),
                engine: TranscriptionEngine::GroqWhisper,
                dual_engine: false,
                deepgram_mode: DeepgramMode::Batch,
                sanitizer: SanitizerModel::Llama70b,
                sanitizer_enabled: false,
                reasoning_enabled: false,
                reasoning_effort: ("medium").into(),
                system_prompt: ("").into(),
                custom_words: vec![],
                source: ("mic").into(),
                duration_ms: 1000,
                copy_to_clipboard: true,
            },
        );
        assert_eq!(req.mode, TranscriptionMode::UltraFast);
        let mocks = MockProviders {
            whisper: Some(Ok("texto do whisper".into())),
            ..Default::default()
        };
        let out = run_mock_pipeline(&req, &mocks);
        assert!(!out.is_error);
        assert_eq!(out.final_text, "texto do whisper");
        assert!(out.deepgram_text.is_none() || out.deepgram_text.as_deref() == Some(""));
    }

    #[test]
    fn mock_sanitizer_empty_fallback() {
        let mut req = PipelineRequest::from_legacy_settings(
            crate::pipeline_contract::FromLegacySettingsInput {
                id: ("m2").into(),
                audio_bytes: vec![],
                file_name: ("a.wav").into(),
                mime: ("audio/wav").into(),
                engine: TranscriptionEngine::GroqWhisper,
                dual_engine: true,
                deepgram_mode: DeepgramMode::Batch,
                sanitizer: SanitizerModel::GptOss120b,
                sanitizer_enabled: true,
                reasoning_enabled: false,
                reasoning_effort: ("medium").into(),
                system_prompt: ("").into(),
                custom_words: vec![],
                source: ("mic").into(),
                duration_ms: 1000,
                copy_to_clipboard: true,
            },
        );
        req.mode = TranscriptionMode::UltraFast;
        let mocks = MockProviders {
            whisper: Some(Ok("whisper raw".into())),
            deepgram: Some(Ok("deepgram raw longer".into())),
            sanitizer: Some(Ok(String::new())),
            ..Default::default()
        };
        let out = run_mock_pipeline(&req, &mocks);
        assert!(!out.is_error);
        assert!(out.used_fallback);
        assert_eq!(out.final_text, "deepgram raw longer");
    }

    #[test]
    fn mock_partial_dual_still_delivers() {
        let req = PipelineRequest::from_legacy_settings(
            crate::pipeline_contract::FromLegacySettingsInput {
                id: ("m3").into(),
                audio_bytes: vec![],
                file_name: ("a.wav").into(),
                mime: ("audio/wav").into(),
                engine: TranscriptionEngine::GroqWhisper,
                dual_engine: true,
                deepgram_mode: DeepgramMode::Batch,
                sanitizer: SanitizerModel::Llama70b,
                sanitizer_enabled: false,
                reasoning_enabled: false,
                reasoning_effort: ("medium").into(),
                system_prompt: ("").into(),
                custom_words: vec![],
                source: ("mic").into(),
                duration_ms: 500,
                copy_to_clipboard: true,
            },
        );
        let mocks = MockProviders {
            whisper: Some(Err("timeout".into())),
            deepgram: Some(Ok("só deepgram".into())),
            ..Default::default()
        };
        let out = run_mock_pipeline(&req, &mocks);
        assert!(!out.is_error);
        assert_eq!(out.final_text, "só deepgram");
        assert!(out
            .journal_warnings
            .iter()
            .any(|w| w.code == "dual_whisper_failed"));
    }

    #[test]
    fn mock_empty_stt_is_error() {
        let req = PipelineRequest::from_legacy_settings(
            crate::pipeline_contract::FromLegacySettingsInput {
                id: ("m4").into(),
                audio_bytes: vec![],
                file_name: ("a.wav").into(),
                mime: ("audio/wav").into(),
                engine: TranscriptionEngine::GroqWhisper,
                dual_engine: false,
                deepgram_mode: DeepgramMode::Batch,
                sanitizer: SanitizerModel::Llama70b,
                sanitizer_enabled: false,
                reasoning_enabled: false,
                reasoning_effort: ("medium").into(),
                system_prompt: ("").into(),
                custom_words: vec![],
                source: ("mic").into(),
                duration_ms: 100,
                copy_to_clipboard: true,
            },
        );
        let mocks = MockProviders {
            whisper: Some(Ok("   ".into())),
            ..Default::default()
        };
        let out = run_mock_pipeline(&req, &mocks);
        assert!(out.is_error);
        assert!(out.error_message.unwrap().contains("Nenhum texto"));
    }

    #[test]
    fn mock_header_only_sanitizer() {
        let mut req = PipelineRequest::from_legacy_settings(
            crate::pipeline_contract::FromLegacySettingsInput {
                id: ("m5").into(),
                audio_bytes: vec![],
                file_name: ("a.wav").into(),
                mime: ("audio/wav").into(),
                engine: TranscriptionEngine::GroqWhisper,
                dual_engine: false,
                deepgram_mode: DeepgramMode::Batch,
                sanitizer: SanitizerModel::Llama70b,
                sanitizer_enabled: true,
                reasoning_enabled: false,
                reasoning_effort: ("medium").into(),
                system_prompt: ("").into(),
                custom_words: vec![],
                source: ("mic").into(),
                duration_ms: 100,
                copy_to_clipboard: true,
            },
        );
        req.mode = TranscriptionMode::UltraFast;
        let mocks = MockProviders {
            whisper: Some(Ok("fala real".into())),
            sanitizer: Some(Ok("## Resumo\nHere is the transcription:".into())),
            ..Default::default()
        };
        let out = run_mock_pipeline(&req, &mocks);
        // After strip, remaining may be empty or residual — must not crash.
        assert!(!out.is_error);
        assert!(!out.final_text.trim().is_empty());
    }

    #[test]
    fn pipeline_result_alias_uses_canonical_transcript_versions() {
        let mut r = PipelineResult::success("99", TranscriptionMode::UltraFast, "duas palavras");
        r.history_engine_label = "GroqWhisper".into();
        r.timings.whisper_ms = Some(50);
        r.timings.sanitizer_ms = Some(30);
        r.timings.recompute_total();
        assert_eq!(r.transcript.raw.as_deref(), Some("duas palavras"));
        assert_eq!(r.transcript.delivered.as_deref(), Some("duas palavras"));
        assert_eq!(r.timings.total_ms, 80);

        let err: PipelineResult =
            PipelineRun::hard_error("100", TranscriptionMode::UltraFast, "falhou");
        assert_eq!(err.status, crate::pipeline_run::PipelineRunStatus::Failed);
        assert_eq!(
            err.error.as_ref().map(|error| error.message.as_str()),
            Some("falhou")
        );
    }

    #[test]
    fn mode_stage_flags() {
        assert!(TranscriptionMode::UltraFast.uses_whisper());
        assert!(!TranscriptionMode::UltraFast.uses_gemini_audio());
        assert!(!TranscriptionMode::UltraFast.uses_sanitizer());

        assert!(!TranscriptionMode::FastAccurate.uses_whisper());
        assert!(TranscriptionMode::FastAccurate.uses_gemini_audio());

        assert!(TranscriptionMode::Precise.uses_whisper());
        assert!(TranscriptionMode::Precise.uses_gemini_audio());
        assert!(!TranscriptionMode::Precise.uses_sanitizer());

        assert!(TranscriptionMode::UltraPrecise.uses_whisper());
        assert!(TranscriptionMode::UltraPrecise.uses_sanitizer());
        assert!(TranscriptionMode::UltraPrecise.uses_gemini_audio());
    }

    #[test]
    fn display_names_pt_are_product_copy() {
        assert_eq!(
            TranscriptionMode::UltraFast.display_name_pt(),
            "Ultrarrápido"
        );
        assert_eq!(
            TranscriptionMode::FastAccurate.display_name_pt(),
            "Rápido e preciso"
        );
        assert_eq!(TranscriptionMode::Precise.display_name_pt(), "Preciso");
        assert_eq!(
            TranscriptionMode::UltraPrecise.display_name_pt(),
            "Ultrapreciso"
        );
    }

    #[test]
    fn pipeline_choice_resolves_presets_and_custom_ids() {
        let preset = GeminiPipelineChoice::default();
        assert_eq!(preset.resolved_model_id().unwrap(), "gemini-3.5-flash-lite");

        let custom_google = GeminiPipelineChoice {
            use_custom_model: true,
            custom_model: " models/gemini-custom-audio ".into(),
            ..Default::default()
        };
        assert_eq!(
            custom_google.resolved_model_id().unwrap(),
            "gemini-custom-audio"
        );

        let custom_openrouter = GeminiPipelineChoice {
            provider: GeminiProvider::OpenRouter,
            use_custom_model: true,
            custom_model: "vendor/custom-audio-model".into(),
            ..Default::default()
        };
        assert_eq!(
            custom_openrouter.resolved_model_id().unwrap(),
            "vendor/custom-audio-model"
        );

        let custom_openrouter_prefixed = GeminiPipelineChoice {
            provider: GeminiProvider::OpenRouter,
            use_custom_model: true,
            custom_model: " models/vendor/custom-audio-model ".into(),
            ..Default::default()
        };
        assert_eq!(
            custom_openrouter_prefixed.resolved_model_id().unwrap(),
            "vendor/custom-audio-model"
        );
    }

    #[test]
    fn whisper_presets_resolve_to_openrouter_model_ids() {
        assert_eq!(
            OpenRouterWhisperModel::LargeV3Turbo.openrouter_id(),
            "openai/whisper-large-v3-turbo"
        );
        assert_eq!(
            OpenRouterWhisperModel::LargeV3.openrouter_id(),
            "openai/whisper-large-v3"
        );
    }

    #[test]
    fn whisper_presets_resolve_to_matching_groq_model_ids() {
        assert_eq!(
            OpenRouterWhisperModel::LargeV3Turbo.groq_id(),
            "whisper-large-v3-turbo"
        );
        assert_eq!(
            OpenRouterWhisperModel::LargeV3.groq_id(),
            "whisper-large-v3"
        );
    }

    #[test]
    fn ultra_fast_provider_defaults_to_openrouter_for_old_installs() {
        assert_eq!(UltraFastProvider::default(), UltraFastProvider::OpenRouter);
        assert_eq!(UltraFastProvider::OpenRouter.as_str(), "open-router");
        assert_eq!(UltraFastProvider::Groq.as_str(), "groq");
        // Old settings.json has no ultra_fast_provider → OpenRouter (current behavior).
        let parsed: GeminiPipelineConfig =
            serde_json::from_str(r#"{"ultra_fast_whisper":"large-v3-turbo"}"#).unwrap();
        assert_eq!(parsed.ultra_fast_provider, UltraFastProvider::OpenRouter);
        assert_eq!(
            parsed.ultra_fast_whisper,
            OpenRouterWhisperModel::LargeV3Turbo
        );
        let parsed_groq: GeminiPipelineConfig = serde_json::from_str(
            r#"{"ultra_fast_provider":"groq","ultra_fast_whisper":"large-v3"}"#,
        )
        .unwrap();
        assert_eq!(parsed_groq.ultra_fast_provider, UltraFastProvider::Groq);
        assert_eq!(
            parsed_groq.ultra_fast_whisper,
            OpenRouterWhisperModel::LargeV3
        );
    }

    #[test]
    fn gemini_transcribe_preset_resolves_and_deserializes() {
        assert_eq!(
            GeminiModel::Transcribe35.google_id(),
            "gemini-3.5-transcribe"
        );
        assert_eq!(
            GeminiModel::Transcribe35.openrouter_id(),
            "google/gemini-3.5-transcribe"
        );

        let parsed: GeminiModel = serde_json::from_str(r#""transcribe35""#).unwrap();
        assert_eq!(parsed, GeminiModel::Transcribe35);

        let parsed_alias: GeminiModel = serde_json::from_str(r#""gemini-3.5-transcribe""#).unwrap();
        assert_eq!(parsed_alias, GeminiModel::Transcribe35);

        let choice = GeminiPipelineChoice {
            model: GeminiModel::Transcribe35,
            provider: GeminiProvider::GoogleAiStudio,
            use_custom_model: false,
            ..Default::default()
        };
        assert_eq!(choice.resolved_model_id().unwrap(), "gemini-3.5-transcribe");
    }

    #[test]
    fn meta_muse_voice_transcribe_resolves_and_deserializes() {
        assert_eq!(
            GeminiModel::MuseVoiceTranscribe.meta_id(),
            "muse-voice-transcribe-1.0"
        );

        let parsed: GeminiModel = serde_json::from_str(r#""muse-voice-transcribe-1.0""#).unwrap();
        assert_eq!(parsed, GeminiModel::MuseVoiceTranscribe);

        let parsed_provider: GeminiProvider = serde_json::from_str(r#""meta""#).unwrap();
        assert_eq!(parsed_provider, GeminiProvider::Meta);

        let choice = GeminiPipelineChoice {
            model: GeminiModel::MuseVoiceTranscribe,
            provider: GeminiProvider::Meta,
            use_custom_model: false,
            ..Default::default()
        };
        assert_eq!(
            choice.resolved_model_id().unwrap(),
            "muse-voice-transcribe-1.0"
        );
    }
}
