//! Small persistent preferences store (`settings.json` in the app data
//! directory). Fields written by older versions that no longer exist here are
//! ignored on load and dropped on the next save.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::OnceLock};

static SETTINGS_PATH: OnceLock<PathBuf> = OnceLock::new();
static SETTINGS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// On-disk representation of the user preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Settings {
    /// Legacy flag from the old permanently-visible gadget. When the new
    /// preference is absent, `true` migrates to `Always`; `false` to `Auto`.
    #[serde(default)]
    compact_mode: bool,
    #[serde(default)]
    widget_visibility_mode: Option<crate::models::WidgetVisibilityMode>,
    #[serde(default)]
    widget_dock: crate::models::WidgetDock,
    /// Stable display identity (Tauri monitor name), never a display index.
    #[serde(default)]
    widget_display: Option<String>,
    #[serde(default)]
    input_device: Option<String>,
    #[serde(default)]
    gadget_x: Option<f64>,
    #[serde(default)]
    gadget_y: Option<f64>,
    /// Physical position (pixels) of the gadget window relative to the
    /// virtual desktop origin. Stored as physical — not logical — so the
    /// restore path is independent of per-monitor scale factors and the
    /// gadget reliably returns to whichever monitor it was last on.
    #[serde(default)]
    gadget_physical_x: Option<i32>,
    #[serde(default)]
    gadget_physical_y: Option<i32>,
    /// Legacy flat word list (pre Phase 06). Migrated into `vocabulary` on load.
    #[serde(default)]
    custom_words: Vec<String>,
    /// Structured vocabulary terms.
    #[serde(default)]
    vocabulary: Vec<crate::vocabulary::VocabularyTerm>,
    /// When `true`, Sonora relaunches itself elevated at startup so it can
    /// paste into apps running as administrator (UIPI blocks it otherwise).
    #[serde(default)]
    run_as_admin: bool,
    #[serde(default)]
    gemini_pipelines: crate::pipeline_contract::GeminiPipelineConfig,
    #[serde(default)]
    smart_context_config: crate::smart_context::Config,
    /// Optional absolute directory for future source-audio files. Existing
    /// history entries keep their own absolute paths when this changes.
    #[serde(default)]
    audio_directory: Option<String>,
    #[serde(default)]
    features: crate::assist::FeatureSettings,
}

/// Called once during setup with the resolved `settings.json` path.
pub fn init(file: PathBuf) {
    let _ = SETTINGS_PATH.set(file);
    let _ = SETTINGS_LOCK.set(Mutex::new(()));
}

fn read() -> Settings {
    let _guard = SETTINGS_LOCK.get_or_init(|| Mutex::new(())).lock();
    SETTINGS_PATH
        .get()
        .map(|file| {
            read_at(file).unwrap_or_else(|error| {
                log::error!("settings: {error}");
                Settings::default()
            })
        })
        .unwrap_or_default()
}
fn update(change: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let _config = crate::models::CONFIG_LOCK.lock();
    let _guard = SETTINGS_LOCK.get_or_init(|| Mutex::new(())).lock();
    let file = SETTINGS_PATH
        .get()
        .ok_or("Diretório de configurações indisponível")?;
    update_at(file, change)
}
fn update_at(file: &std::path::Path, change: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let mut settings = read_at(file)?;
    change(&mut settings);
    crate::storage::write_json(file, &settings)
}
fn read_at(file: &std::path::Path) -> Result<Settings, String> {
    if !file.exists() {
        return Ok(Settings::default());
    }
    let value: serde_json::Value = crate::storage::read_json(file)?;
    if !value.is_object() {
        return Err(
            "Configurações inválidas: esperado um objeto JSON. Original preservado.".into(),
        );
    }
    serde_json::from_value(value).map_err(|_| "Configurações inválidas; restaure um backup".into())
}

/// Returns the persisted compact-mode flag (defaults to `false`).
pub fn load_compact() -> bool {
    read().compact_mode
}

/// Returns the persisted input-device selection (defaults to `None`).
pub fn load_input_device() -> Option<String> {
    read().input_device
}

/// Persists the input-device selection, preserving any other settings.
pub fn save_input_device(device: Option<String>) -> Result<(), String> {
    update(|s| {
        s.input_device = device;
    })
}

/// Loads structured vocabulary, migrating legacy `custom_words` if needed.
pub fn load_vocabulary() -> Vec<crate::vocabulary::VocabularyTerm> {
    let s = read();
    let base = if !s.vocabulary.is_empty() {
        s.vocabulary
    } else if s.custom_words.is_empty() {
        Vec::new()
    } else {
        // One-shot migration: convert flat list and persist structured form.
        let migrated = crate::vocabulary::migrate_from_strings(&s.custom_words);
        log::info!(
            "settings: migrated {} legacy custom_words → structured vocabulary",
            migrated.len()
        );
        migrated
    };

    crate::vocabulary::ensure_default_product_terms(base)
}

/// Persists structured vocabulary and mirrors enabled canonicals into legacy field.
pub fn save_vocabulary(terms: Vec<crate::vocabulary::VocabularyTerm>) -> Result<(), String> {
    update(|s| {
        s.custom_words = crate::vocabulary::canonical_list(&terms);
        s.vocabulary = terms;
    })
}

/// Returns the persisted gadget window position in **physical** pixels
/// relative to the virtual desktop origin. Used by the gadget setup so
/// the overlay reappears on the exact monitor (and spot) it was last
/// dragged to, regardless of per-monitor scale factors.
pub fn load_gadget_physical_position() -> Option<(i32, i32)> {
    let s = read();
    match (s.gadget_physical_x, s.gadget_physical_y) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    }
}

/// Returns the persisted run-as-administrator flag (defaults to `false`).
pub fn load_run_as_admin() -> bool {
    read().run_as_admin
}

/// Persists the run-as-administrator flag, preserving any other settings.
pub fn save_run_as_admin(value: bool) -> Result<(), String> {
    update(|s| {
        s.run_as_admin = value;
    })
}

/// Persists the single transcription route (always the Fast & Accurate mode).
pub fn save_transcription_route(
    route: crate::pipeline_contract::GeminiPipelineChoice,
) -> Result<(), String> {
    update(|s| {
        s.gemini_pipelines.fast_accurate = route;
    })
}

/// Loads the new visibility preference, coherently migrating the old compact
/// flag instead of exposing two contradictory settings.
pub fn load_widget_visibility_mode() -> crate::models::WidgetVisibilityMode {
    let s = read();
    s.widget_visibility_mode.unwrap_or(if s.compact_mode {
        crate::models::WidgetVisibilityMode::Always
    } else {
        crate::models::WidgetVisibilityMode::Auto
    })
}

pub fn save_widget_visibility_mode(
    value: crate::models::WidgetVisibilityMode,
) -> Result<(), String> {
    update(|s| {
        s.widget_visibility_mode = Some(value);
        // Keep the legacy field meaningful for one-version backwards compatibility.
        s.compact_mode = value == crate::models::WidgetVisibilityMode::Always;
    })
}

pub fn load_widget_dock() -> crate::models::WidgetDock {
    read().widget_dock
}

pub fn load_widget_display() -> Option<String> {
    read()
        .widget_display
        .filter(|value| !value.trim().is_empty())
}

pub fn load_gemini_pipelines() -> crate::pipeline_contract::GeminiPipelineConfig {
    read().gemini_pipelines.clone()
}

pub fn load_smart_context_config() -> crate::smart_context::Config {
    read().smart_context_config
}

pub fn save_smart_context_config(config: crate::smart_context::Config) -> Result<(), String> {
    update(|s| s.smart_context_config = config)
}

pub fn load_audio_directory() -> Option<String> {
    read()
        .audio_directory
        .filter(|path| !path.trim().is_empty())
}

pub fn load_features() -> crate::assist::FeatureSettings {
    read().features
}

pub fn save_features(features: &crate::assist::FeatureSettings) -> Result<(), String> {
    update(|s| s.features = features.clone())
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn concurrent_preferences_keep_both_changes_and_reject_corruption() {
        let dir = std::env::temp_dir().join(format!("sonora-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        std::thread::scope(|scope| {
            for index in 0..20 {
                let file = &file;
                scope.spawn(move || {
                    let _guard = SETTINGS_LOCK.get_or_init(|| Mutex::new(())).lock();
                    update_at(file, |settings| {
                        if index % 2 == 0 {
                            settings.input_device = Some("synthetic mic".into());
                        } else {
                            settings.widget_display = Some("synthetic display".into());
                        }
                    })
                    .unwrap();
                });
            }
        });
        let saved: Settings = crate::storage::read_json(&file).unwrap();
        assert_eq!(saved.input_device.as_deref(), Some("synthetic mic"));
        assert_eq!(saved.widget_display.as_deref(), Some("synthetic display"));
        std::fs::write(&file, b"[]").unwrap();
        assert!(update_at(&file, |settings| settings.run_as_admin = true).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"[]");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
