//! Stable product-mode identifiers and the persisted transcription route.
//!
//! Only "Rápido e preciso" runs today; the other mode ids stay deserializable
//! so history written by earlier versions keeps loading and labelling.

use serde::{Deserialize, Deserializer, Serialize};

// ─── Modes & content ────────────────────────────────────────────────────────

/// Product-facing transcription mode.
///
/// Wire ids are stable kebab-case for settings.json / IPC.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TranscriptionMode {
    /// Whisper through OpenRouter's dedicated STT endpoint.
    #[default]
    UltraFast,
    /// Primary configured audio transcription route.
    FastAccurate,
    /// Legacy multi-stage mode (kept for history deserialization).
    Precise,
    /// Legacy multi-stage mode (kept for history deserialization).
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
}

/// Optional content hint for future prompt routing.
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

// ─── Transcription provider & model catalog ─────────────────────────────────

/// Supported transcription providers:
/// - `AssemblyAi`: direct AssemblyAI pre-recorded API (`assemblyai-stt-3-7`)
/// - `OpenRouter`: OpenRouter transcription models + any compatible custom ID
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TranscriptionProvider {
    #[default]
    #[serde(rename = "assemblyai")]
    AssemblyAi,
    OpenRouter,
}

impl<'de> Deserialize<'de> for TranscriptionProvider {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "assemblyai" | "assembly-ai" => Ok(Self::AssemblyAi),
            "open-router" | "openrouter" | "meta" => Ok(Self::OpenRouter),
            // Legacy Google AI Studio installs migrate safely to direct AssemblyAI.
            "google-ai-studio" => Ok(Self::AssemblyAi),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &["assemblyai", "open-router"],
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
pub enum TranscriptionModel {
    /// AssemblyAI-STT-3.7 via direct AssemblyAI pre-recorded API.
    #[default]
    #[serde(rename = "assemblyai-stt-3-7")]
    AssemblyAiStt37,
    /// AssemblyAI Universal-3.6 Pro via OpenRouter STT endpoint.
    #[serde(rename = "assemblyai-universal-3-6-pro")]
    AssemblyAiUniversal36Pro,
    /// OpenAI Whisper Large v3 via OpenRouter STT endpoint.
    #[serde(rename = "openai-whisper-large-v3")]
    OpenAiWhisperLargeV3,
    /// OpenAI Whisper Large v3 Turbo via OpenRouter STT endpoint.
    #[serde(rename = "openai-whisper-large-v3-turbo")]
    OpenAiWhisperLargeV3Turbo,
    /// OpenAI GPT-4o Transcribe via OpenRouter STT endpoint.
    #[serde(rename = "openai-gpt-4o-transcribe")]
    OpenAiGpt4oTranscribe,
    /// OpenAI GPT-4o Mini Transcribe via OpenRouter STT endpoint.
    #[serde(rename = "openai-gpt-4o-mini-transcribe")]
    OpenAiGpt4oMiniTranscribe,
}

impl<'de> Deserialize<'de> for TranscriptionModel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "assemblyai-stt-3-7" => Ok(Self::AssemblyAiStt37),
            "assemblyai-universal-3-6-pro" => Ok(Self::AssemblyAiUniversal36Pro),
            "openai-whisper-large-v3" => Ok(Self::OpenAiWhisperLargeV3),
            "openai-whisper-large-v3-turbo" => Ok(Self::OpenAiWhisperLargeV3Turbo),
            "openai-gpt-4o-transcribe" => Ok(Self::OpenAiGpt4oTranscribe),
            "openai-gpt-4o-mini-transcribe" => Ok(Self::OpenAiGpt4oMiniTranscribe),
            // Legacy Gemini/Meta presets migrate safely without breaking old settings.json.
            "flash-lite35"
            | "flash36"
            | "transcribe35"
            | "gemini-3.5-transcribe"
            | "transcribe-35"
            | "muse-voice-transcribe-1.0"
            | "muse-voice-transcribe" => Ok(Self::AssemblyAiUniversal36Pro),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &[
                    "assemblyai-stt-3-7",
                    "assemblyai-universal-3-6-pro",
                    "openai-whisper-large-v3",
                    "openai-whisper-large-v3-turbo",
                    "openai-gpt-4o-transcribe",
                    "openai-gpt-4o-mini-transcribe",
                ],
            )),
        }
    }
}

impl TranscriptionModel {
    pub fn api_id(self) -> &'static str {
        match self {
            Self::AssemblyAiStt37 => crate::assemblyai::MODEL_STT_3_7,
            Self::AssemblyAiUniversal36Pro => "assemblyai/universal-3-6-pro",
            Self::OpenAiWhisperLargeV3 => "openai/whisper-large-v3",
            Self::OpenAiWhisperLargeV3Turbo => "openai/whisper-large-v3-turbo",
            Self::OpenAiGpt4oTranscribe => "openai/gpt-4o-transcribe",
            Self::OpenAiGpt4oMiniTranscribe => "openai/gpt-4o-mini-transcribe",
        }
    }

    pub fn is_openrouter_catalog(self) -> bool {
        !matches!(self, Self::AssemblyAiStt37)
    }
}

/// Validates a user-entered custom OpenRouter model ID (`vendor/model`).
pub fn validate_custom_openrouter_model(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Informe um ID de modelo do OpenRouter (ex.: openai/whisper-large-v3).".into());
    }
    if trimmed.len() > 140 {
        return Err("ID do modelo OpenRouter muito longo (máximo de 140 caracteres).".into());
    }
    let Some((vendor, name)) = trimmed.split_once('/') else {
        return Err(
            "Use o formato provedor/modelo do OpenRouter (ex.: openai/whisper-large-v3).".into(),
        );
    };
    if vendor.trim().is_empty() || name.trim().is_empty() {
        return Err("ID do modelo OpenRouter inválido.".into());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | ':'))
    {
        return Err(
            "ID do modelo OpenRouter contém caracteres inválidos (use apenas letras, números, '/', '-', '_', '.' ou ':')."
                .into(),
        );
    }
    Ok(trimmed.to_string())
}

/// The audio model route used for every dictation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TranscriptionRoute {
    pub model: TranscriptionModel,
    pub provider: TranscriptionProvider,
    #[serde(default)]
    pub use_custom_model: bool,
    #[serde(default)]
    pub custom_model: String,
}

impl<'de> Deserialize<'de> for TranscriptionRoute {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Stored {
            #[serde(default)]
            model: TranscriptionModel,
            #[serde(default)]
            provider: TranscriptionProvider,
            #[serde(default)]
            use_custom_model: bool,
            #[serde(default)]
            custom_model: String,
        }
        let stored = Stored::deserialize(deserializer)?;
        let custom_trimmed = stored.custom_model.trim().to_string();
        let mut route = Self {
            model: stored.model,
            provider: stored.provider,
            use_custom_model: stored.use_custom_model && !custom_trimmed.is_empty(),
            custom_model: custom_trimmed,
        };
        route.normalize();
        Ok(route)
    }
}

impl TranscriptionRoute {
    /// Ensures provider, model, and custom ID flags are mutually consistent.
    pub fn normalize(&mut self) {
        self.custom_model = self.custom_model.trim().to_string();
        match self.provider {
            TranscriptionProvider::AssemblyAi => {
                self.model = TranscriptionModel::AssemblyAiStt37;
                self.use_custom_model = false;
            }
            TranscriptionProvider::OpenRouter => {
                if !self.model.is_openrouter_catalog() {
                    self.model = TranscriptionModel::AssemblyAiUniversal36Pro;
                }
                if self.use_custom_model {
                    // If a previously saved custom id is no longer valid, fall back cleanly.
                    if validate_custom_openrouter_model(&self.custom_model).is_err() {
                        self.use_custom_model = false;
                    }
                }
            }
        }
    }

    /// Validates user-submitted route changes (rejects invalid custom OpenRouter IDs).
    pub fn validate(mut self) -> Result<Self, String> {
        self.custom_model = self.custom_model.trim().to_string();
        if self.provider == TranscriptionProvider::OpenRouter && self.use_custom_model {
            self.custom_model = validate_custom_openrouter_model(&self.custom_model)?;
        }
        self.normalize();
        Ok(self)
    }

    pub fn model_id(&self) -> &str {
        if self.provider == TranscriptionProvider::OpenRouter
            && self.use_custom_model
            && !self.custom_model.trim().is_empty()
        {
            self.custom_model.trim()
        } else {
            self.model.api_id()
        }
    }
}

/// Persisted transcription route container. Keeps `fast_accurate` wire name
/// so existing `settings.json` files load without migration loss.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptionPipelineConfig {
    #[serde(default)]
    pub fast_accurate: TranscriptionRoute,
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(serde_json::from_str::<TranscriptionMode>(r#""teleport""#).is_err());
    }

    #[test]
    fn unknown_content_type_fails_closed() {
        assert!(serde_json::from_str::<ContentType>(r#""podcast""#).is_err());
    }

    #[test]
    fn default_route_uses_direct_assemblyai_stt_3_7() {
        let preset = TranscriptionRoute::default();
        assert_eq!(preset.provider, TranscriptionProvider::AssemblyAi);
        assert_eq!(preset.model, TranscriptionModel::AssemblyAiStt37);
        assert_eq!(preset.model_id(), "assemblyai-stt-3-7");
        assert!(!preset.use_custom_model);
    }

    #[test]
    fn openrouter_catalog_excludes_gemini_and_supports_universal_3_6_and_custom_ids() {
        let universal = TranscriptionRoute {
            provider: TranscriptionProvider::OpenRouter,
            model: TranscriptionModel::AssemblyAiUniversal36Pro,
            use_custom_model: false,
            custom_model: String::new(),
        };
        assert_eq!(universal.model_id(), "assemblyai/universal-3-6-pro");

        let custom = TranscriptionRoute {
            provider: TranscriptionProvider::OpenRouter,
            model: TranscriptionModel::AssemblyAiUniversal36Pro,
            use_custom_model: true,
            custom_model: "  mistralai/voxtral-small-24b-2507  ".into(),
        }
        .validate()
        .unwrap();
        assert!(custom.use_custom_model);
        assert_eq!(custom.model_id(), "mistralai/voxtral-small-24b-2507");

        // Round-trip custom OpenRouter ID through JSON.
        let json = serde_json::to_string(&custom).unwrap();
        let back: TranscriptionRoute = serde_json::from_str(&json).unwrap();
        assert_eq!(back, custom);

        // Reject malformed custom IDs.
        assert!(TranscriptionRoute {
            provider: TranscriptionProvider::OpenRouter,
            model: TranscriptionModel::OpenAiWhisperLargeV3,
            use_custom_model: true,
            custom_model: "not-a-valid-id".into(),
        }
        .validate()
        .is_err());
    }

    #[test]
    fn legacy_google_and_gemini_settings_migrate_cleanly() {
        let google_json = r#"{"model": "transcribe35", "provider": "google-ai-studio"}"#;
        let parsed: TranscriptionRoute = serde_json::from_str(google_json).unwrap();
        assert_eq!(parsed.provider, TranscriptionProvider::AssemblyAi);
        assert_eq!(parsed.model, TranscriptionModel::AssemblyAiStt37);

        let openrouter_gemini_json = r#"{
            "fast_accurate": {"model": "flash36", "provider": "open-router", "use_custom_model": false, "custom_model": ""}
        }"#;
        let parsed_cfg: TranscriptionPipelineConfig =
            serde_json::from_str(openrouter_gemini_json).unwrap();
        assert_eq!(
            parsed_cfg.fast_accurate.provider,
            TranscriptionProvider::OpenRouter
        );
        assert_eq!(
            parsed_cfg.fast_accurate.model,
            TranscriptionModel::AssemblyAiUniversal36Pro
        );
        assert_eq!(
            parsed_cfg.fast_accurate.model_id(),
            "assemblyai/universal-3-6-pro"
        );
    }

    #[test]
    fn removed_meta_route_migrates_to_openrouter_universal_3_6() {
        let parsed: TranscriptionRoute =
            serde_json::from_str(r#"{"model": "muse-voice-transcribe-1.0", "provider": "meta"}"#)
                .unwrap();
        assert_eq!(parsed.provider, TranscriptionProvider::OpenRouter);
        assert_eq!(parsed.model, TranscriptionModel::AssemblyAiUniversal36Pro);
    }
}
