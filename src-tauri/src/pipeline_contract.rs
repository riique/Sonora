//! Stable product-mode identifiers and the persisted transcription route.
//!
//! Only "Rápido e preciso" runs today; the other mode ids stay deserializable
//! so history written by earlier versions keeps loading and labelling.

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
    /// Older installs may still carry the removed Meta model; they fall back
    /// to the default preset.
    #[default]
    #[serde(alias = "muse-voice-transcribe-1.0", alias = "muse-voice-transcribe")]
    FlashLite35,
    Flash36,
    #[serde(alias = "gemini-3.5-transcribe", alias = "transcribe-35")]
    Transcribe35,
    /// AssemblyAI Universal-3.6 Pro. Served only through OpenRouter's
    /// speech-to-text endpoint (vocabulary and smart context as keyterms).
    #[serde(rename = "assemblyai-universal-3-6-pro")]
    AssemblyAiUniversal36Pro,
}

impl GeminiModel {
    pub fn google_id(self) -> &'static str {
        match self {
            Self::FlashLite35 => "gemini-3.5-flash-lite",
            Self::Flash36 => "gemini-3.6-flash",
            Self::Transcribe35 => "gemini-3.5-transcribe",
            // Not offered by Google AI Studio; the UI never pairs them.
            Self::AssemblyAiUniversal36Pro => "gemini-3.5-transcribe",
        }
    }

    pub fn openrouter_id(self) -> &'static str {
        match self {
            Self::FlashLite35 => "google/gemini-3.5-flash-lite",
            Self::Flash36 => "google/gemini-3.6-flash",
            Self::Transcribe35 => "google/gemini-3.5-transcribe",
            Self::AssemblyAiUniversal36Pro => "assemblyai/universal-3-6-pro",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GeminiProvider {
    #[default]
    GoogleAiStudio,
    /// The removed Meta provider migrates to OpenRouter.
    #[serde(alias = "meta")]
    OpenRouter,
}

/// The audio model used for every dictation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GeminiPipelineChoice {
    pub model: GeminiModel,
    pub provider: GeminiProvider,
}

/// Older versions stored free-form model ids. The AssemblyAI id maps to its
/// preset; any other custom id falls back to the saved preset. Meta-only
/// fields are ignored.
impl<'de> Deserialize<'de> for GeminiPipelineChoice {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Stored {
            #[serde(default)]
            model: GeminiModel,
            #[serde(default)]
            provider: GeminiProvider,
            #[serde(default)]
            use_custom_model: bool,
            #[serde(default)]
            custom_model: String,
        }
        let stored = Stored::deserialize(deserializer)?;
        let custom = stored.custom_model.trim().to_ascii_lowercase();
        if stored.use_custom_model && custom.starts_with("assemblyai/universal-3") {
            return Ok(Self {
                model: GeminiModel::AssemblyAiUniversal36Pro,
                provider: GeminiProvider::OpenRouter,
            });
        }
        let mut choice = Self {
            model: stored.model,
            provider: stored.provider,
        };
        choice.normalize();
        Ok(choice)
    }
}

impl GeminiPipelineChoice {
    /// AssemblyAI exists only on OpenRouter.
    pub fn normalize(&mut self) {
        if self.model == GeminiModel::AssemblyAiUniversal36Pro {
            self.provider = GeminiProvider::OpenRouter;
        }
    }

    pub fn model_id(&self) -> &'static str {
        match self.provider {
            GeminiProvider::GoogleAiStudio => self.model.google_id(),
            GeminiProvider::OpenRouter => self.model.openrouter_id(),
        }
    }
}

/// Persisted transcription route. Older settings files also carry routes for
/// the removed modes; serde ignores them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeminiPipelineConfig {
    #[serde(default)]
    pub fast_accurate: GeminiPipelineChoice,
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
        let err = serde_json::from_str::<TranscriptionMode>(r#""teleport""#);
        assert!(err.is_err());
    }

    #[test]
    fn unknown_content_type_fails_closed() {
        let err = serde_json::from_str::<ContentType>(r#""podcast""#);
        assert!(err.is_err());
    }

    #[test]
    fn pipeline_choice_resolves_provider_model_ids() {
        let preset = GeminiPipelineChoice::default();
        assert_eq!(preset.model_id(), "gemini-3.5-flash-lite");
        let routed = GeminiPipelineChoice {
            model: GeminiModel::Transcribe35,
            provider: GeminiProvider::OpenRouter,
        };
        assert_eq!(routed.model_id(), "google/gemini-3.5-transcribe");
    }

    #[test]
    fn old_settings_with_removed_routes_still_load() {
        let json = r#"{
            "ultra_fast_provider": "groq",
            "ultra_fast_whisper": "large-v3",
            "fast_accurate": {"model": "transcribe35", "provider": "open-router", "use_custom_model": false, "custom_model": "", "meta_languages": ["Portuguese"]},
            "precise": {"model": "flash-lite35", "provider": "google-ai-studio"},
            "ultra_precise": {"model": "flash36", "provider": "google-ai-studio"}
        }"#;
        let parsed: GeminiPipelineConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.fast_accurate.model, GeminiModel::Transcribe35);
        assert_eq!(parsed.fast_accurate.provider, GeminiProvider::OpenRouter);
    }

    #[test]
    fn assemblyai_custom_id_migrates_to_its_preset() {
        let parsed: GeminiPipelineChoice = serde_json::from_str(
            r#"{"model": "transcribe35", "provider": "open-router", "use_custom_model": true, "custom_model": "assemblyai/universal-3-6-pro"}"#,
        )
        .unwrap();
        assert_eq!(parsed.model, GeminiModel::AssemblyAiUniversal36Pro);
        assert_eq!(parsed.model_id(), "assemblyai/universal-3-6-pro");
        let json = serde_json::to_string(&parsed).unwrap();
        let back: GeminiPipelineChoice = serde_json::from_str(&json).unwrap();
        assert_eq!(back, parsed);

        let other_custom: GeminiPipelineChoice = serde_json::from_str(
            r#"{"model": "flash36", "provider": "google-ai-studio", "use_custom_model": true, "custom_model": "gemini-9-flash"}"#,
        )
        .unwrap();
        assert_eq!(other_custom.model, GeminiModel::Flash36);
        assert_eq!(other_custom.provider, GeminiProvider::GoogleAiStudio);
    }

    #[test]
    fn removed_meta_route_migrates_to_openrouter_default_model() {
        let parsed: GeminiPipelineChoice =
            serde_json::from_str(r#"{"model": "muse-voice-transcribe-1.0", "provider": "meta"}"#)
                .unwrap();
        assert_eq!(parsed.provider, GeminiProvider::OpenRouter);
        assert_eq!(parsed.model, GeminiModel::FlashLite35);
    }
}
