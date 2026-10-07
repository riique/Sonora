use cpal::Stream;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Deserializer, Serialize};
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};

/// Sanitizer model selected manually by the user from the UI.
/// Each variant maps to a cloud-hosted LLM used for semantic validation
/// of the transcribed text before final delivery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SanitizerModel {
    #[serde(rename = "llama-70b")]
    #[default]
    Llama70b,
    #[serde(rename = "gpt-oss-20b")]
    GptOss20b,
    #[serde(rename = "gpt-oss-120b")]
    GptOss120b,
    #[serde(rename = "qwen3-27b")]
    Qwen327b,
}

impl SanitizerModel {
    /// Maps the user-facing enum variant to the exact model string
    /// expected by the Groq / OpenAI Chat Completions endpoint.
    ///
    /// The GPT-OSS models are published on GroqCloud under the `openai/`
    /// namespace; the bare `gpt-oss-*` ids return `model_not_found`, which
    /// would silently fall the pipeline back to the raw transcription.
    pub fn api_model_id(&self) -> &'static str {
        match self {
            Self::Llama70b => "llama-3.3-70b-versatile",
            Self::GptOss20b => "openai/gpt-oss-20b",
            Self::GptOss120b => "openai/gpt-oss-120b",
            Self::Qwen327b => "qwen/qwen3.6-27b",
        }
    }

    /// Whether this model honours Groq's native `reasoning_effort` request
    /// parameter. Only the GPT-OSS family accepts `low`/`medium`/`high`;
    /// LLaMA 3.3 70B and the Qwen 3.6 27B preview model on Groq have no
    /// documented native reasoning control we can safely send, so enabling
    /// reasoning for them is a no-op (and sending the parameter would risk
    /// an API rejection that silently falls the pipeline back to raw text).
    pub fn supports_reasoning(&self) -> bool {
        matches!(self, Self::GptOss20b | Self::GptOss120b)
    }
}

/// Rectangle of the gadget overlay's *visible pill*, reported by the frontend
/// in logical pixels relative to the gadget window's top-left corner.
///
/// The gadget window is a fixed, mostly-transparent box; without this the whole
/// box would swallow mouse clicks. The background cursor watcher (see
/// `lib.rs`) uses this rect to keep the window click-through everywhere except
/// over the pill itself.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GadgetHitRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Controls whether the dictation bar exists while no dictation is active.
/// `Auto` is intentionally the default: Sonora should disappear until needed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetVisibilityMode {
    #[default]
    Auto,
    Always,
}

/// Dock is persisted as a stable semantic value instead of a screen index.
/// The current implementation deliberately ships the robust bottom anchor;
/// side docking can be added later without migrating the settings format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetDock {
    #[default]
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetPreferences {
    pub visibility_mode: WidgetVisibilityMode,
    pub dock: WidgetDock,
    pub display: Option<String>,
}

/// Explicit presentation states shared by the React state machine and the
/// native window controller. Geometry and native visibility are derived from
/// this enum in one place (`lib.rs`), never from independent booleans.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GadgetVisualState {
    #[default]
    Hidden,
    Idle,
    Hover,
    Appearing,
    Initializing,
    Recording,
    Stopping,
    Processing,
    ProcessingLong,
    Success,
    NoSpeech,
    Error,
}

/// Native presentation generation returned to the gadget frontend. The
/// frontend acknowledges this exact generation after its visible pill has
/// completed layout, allowing the native controller to reject stale paint
/// reports and recover a renderer that stopped presenting frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GadgetPresentation {
    pub visual_state: GadgetVisualState,
    pub generation: u64,
}

/// Authoritative recording lifecycle phase shared with every frontend window.
/// A session remains `stopping` until its transcription has finished, so a
/// second capture cannot reuse the shared stream/buffer while the previous
/// pipeline is still consuming them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingPhase {
    #[default]
    Idle,
    Starting,
    Recording,
    Stopping,
    Cancelling,
}

/// Monotonic backend truth sent with each recording lifecycle event.
/// `revision` orders transitions even within the same session, closing the
/// snapshot/listener race in React; `session_id` correlates pipeline results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingStatus {
    pub generation: u64,
    pub revision: u64,
    pub session_id: Option<String>,
    pub phase: RecordingPhase,
    pub recording: bool,
    pub busy: bool,
}

#[derive(Debug, Default)]
struct RecordingLifecycle {
    generation: u64,
    revision: u64,
    session_id: Option<String>,
    phase: RecordingPhase,
    capture_start_pending: bool,
}

impl RecordingLifecycle {
    fn snapshot(&self) -> RecordingStatus {
        RecordingStatus {
            generation: self.generation,
            revision: self.revision,
            session_id: self.session_id.clone(),
            phase: self.phase,
            recording: matches!(
                self.phase,
                RecordingPhase::Starting | RecordingPhase::Recording
            ),
            busy: self.phase != RecordingPhase::Idle,
        }
    }

    fn begin(&mut self, session_id: String) -> Option<RecordingStatus> {
        if self.phase != RecordingPhase::Idle {
            return None;
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        self.revision = self.revision.wrapping_add(1).max(1);
        self.session_id = Some(session_id);
        self.phase = RecordingPhase::Starting;
        self.capture_start_pending = true;
        Some(self.snapshot())
    }

    fn request_stop(&mut self) -> Option<RecordingStatus> {
        if !matches!(
            self.phase,
            RecordingPhase::Starting | RecordingPhase::Recording
        ) {
            return None;
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        self.phase = RecordingPhase::Stopping;
        Some(self.snapshot())
    }

    fn capture_ready(&mut self, generation: u64) -> Option<(bool, RecordingStatus)> {
        if self.generation != generation || !self.capture_start_pending {
            return None;
        }
        self.capture_start_pending = false;
        self.revision = self.revision.wrapping_add(1).max(1);
        let accepted = self.phase == RecordingPhase::Starting;
        if accepted {
            self.phase = RecordingPhase::Recording;
        }
        Some((accepted, self.snapshot()))
    }

    fn capture_failed(&mut self, generation: u64) -> Option<RecordingStatus> {
        if self.generation != generation || !self.capture_start_pending {
            return None;
        }
        self.capture_start_pending = false;
        self.revision = self.revision.wrapping_add(1).max(1);
        if matches!(
            self.phase,
            RecordingPhase::Starting | RecordingPhase::Cancelling
        ) {
            self.phase = RecordingPhase::Idle;
            self.session_id = None;
        }
        Some(self.snapshot())
    }

    fn request_cancel(&mut self) -> RecordingStatus {
        if matches!(
            self.phase,
            RecordingPhase::Starting | RecordingPhase::Recording
        ) {
            self.revision = self.revision.wrapping_add(1).max(1);
            self.phase = RecordingPhase::Cancelling;
        }
        self.snapshot()
    }

    fn finish_cancel(&mut self, generation: u64) -> RecordingStatus {
        if self.generation == generation
            && self.phase == RecordingPhase::Cancelling
            && !self.capture_start_pending
        {
            self.revision = self.revision.wrapping_add(1).max(1);
            self.phase = RecordingPhase::Idle;
            self.session_id = None;
        }
        self.snapshot()
    }

    fn finish_stop(&mut self, generation: u64) -> RecordingStatus {
        if self.generation == generation && self.phase == RecordingPhase::Stopping {
            self.revision = self.revision.wrapping_add(1).max(1);
            self.phase = RecordingPhase::Idle;
            self.session_id = None;
            self.capture_start_pending = false;
        }
        self.snapshot()
    }
}

#[cfg(test)]
mod recording_lifecycle_tests {
    use super::{RecordingLifecycle, RecordingPhase};

    #[test]
    fn rapid_restart_is_rejected_until_previous_pipeline_finishes() {
        let mut lifecycle = RecordingLifecycle::default();
        let started = lifecycle.begin("session-1".into()).unwrap();
        assert_eq!(started.phase, RecordingPhase::Starting);
        let generation = started.generation;

        let stopping = lifecycle.request_stop().unwrap();
        assert_eq!(stopping.phase, RecordingPhase::Stopping);
        assert!(!stopping.recording);
        assert!(stopping.busy);
        assert!(lifecycle.begin("session-2".into()).is_none());

        let (accepted, after_start_worker) = lifecycle.capture_ready(generation).unwrap();
        assert!(!accepted);
        assert_eq!(after_start_worker.phase, RecordingPhase::Stopping);
        assert!(lifecycle.begin("session-2".into()).is_none());

        let idle = lifecycle.finish_stop(generation);
        assert_eq!(idle.phase, RecordingPhase::Idle);
        assert!(!idle.busy);

        let second = lifecycle.begin("session-2".into()).unwrap();
        assert!(second.generation > generation);
        assert!(second.revision > idle.revision);
        assert!(lifecycle.capture_ready(generation).is_none());
        assert_eq!(
            lifecycle.snapshot().session_id.as_deref(),
            Some("session-2")
        );
    }

    #[test]
    fn cancel_during_start_waits_for_start_worker_before_becoming_idle() {
        let mut lifecycle = RecordingLifecycle::default();
        let started = lifecycle.begin("session-1".into()).unwrap();
        let cancelling = lifecycle.request_cancel();
        assert_eq!(cancelling.phase, RecordingPhase::Cancelling);
        assert!(lifecycle.finish_cancel(started.generation).busy);

        let failed = lifecycle.capture_failed(started.generation).unwrap();
        assert_eq!(failed.phase, RecordingPhase::Idle);
        assert!(!failed.busy);
    }
}

pub(crate) enum RecordingToggle {
    Start(RecordingStatus),
    Stop(RecordingStatus),
    Busy(RecordingStatus),
}

/// Current work-area anchor in physical pixels. It follows the foreground
/// application's monitor while the gadget is visible; configured display,
/// cursor and primary monitor are fallbacks.
#[derive(Debug, Clone)]
pub struct GadgetSessionAnchor {
    pub display_name: Option<String>,
    pub work_x: i32,
    pub work_y: i32,
    pub work_width: u32,
    pub work_height: u32,
    pub scale: f64,
}

/// Active transcription engine selected manually by the user from the UI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TranscriptionEngine {
    #[default]
    GroqWhisper,
    DeepgramNova3,
    GeminiMultimodal,
}

/// Deepgram delivery mode. Controls how audio is sent to Deepgram when the
/// Nova-3 engine is selected (including dual-engine mode).
///
/// * [`Batch`] — classic REST upload of the complete file (current default).
/// * [`StreamingFinal`] — WebSocket streaming with `interim_results=false`;
///   only the final transcript is returned after the full audio has been
///   pushed (no partials reach the UI). Processing starts incrementally as
///   chunks arrive, typically reducing time-to-final vs batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeepgramMode {
    #[default]
    Batch,
    StreamingFinal,
}

impl DeepgramMode {
    /// Wire-stable identifier used in logs and UI (`"batch"` / `"streaming_final"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Batch => "batch",
            Self::StreamingFinal => "streaming_final",
        }
    }
}

/// API keys for the supported cloud engines. Held in memory (RAM) for the
/// running session and also persisted to `api_keys.json` in the app data
/// directory (see the `secrets` module) so they survive an app restart.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiKeys {
    #[serde(default, deserialize_with = "deserialize_key_list")]
    pub groq: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_key_list")]
    pub google: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_key_list")]
    pub deepgram: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_key_list")]
    pub openrouter: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_key_list")]
    pub meta: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredKeyList {
    One(String),
    Many(Vec<String>),
}

fn deserialize_key_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let stored = Option::<StoredKeyList>::deserialize(deserializer)?;
    Ok(match stored {
        Some(StoredKeyList::One(key)) => vec![key],
        Some(StoredKeyList::Many(keys)) => keys,
        None => Vec::new(),
    })
}

fn normalized_keys(keys: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for key in keys {
        let key = key.trim().to_string();
        if !key.is_empty() && !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

impl ApiKeys {
    pub fn normalized(self) -> Self {
        Self {
            groq: normalized_keys(self.groq),
            google: normalized_keys(self.google),
            deepgram: normalized_keys(self.deepgram),
            openrouter: normalized_keys(self.openrouter),
            meta: normalized_keys(self.meta),
        }
    }
}

#[cfg(test)]
mod api_keys_tests {
    use std::sync::atomic::AtomicUsize;

    use super::{ApiKeys, AppState};

    #[test]
    fn migrates_legacy_single_keys_to_lists() {
        let keys: ApiKeys =
            serde_json::from_str(r#"{"groq":"g-one","google":"a-one","deepgram":null}"#).unwrap();
        assert_eq!(keys.groq, vec!["g-one"]);
        assert_eq!(keys.google, vec!["a-one"]);
        assert!(keys.deepgram.is_empty());
    }

    #[test]
    fn normalizes_lists_without_losing_order() {
        let keys: ApiKeys =
            serde_json::from_str(r#"{"google":[" first ","second","first",""]}"#).unwrap();
        assert_eq!(keys.normalized().google, vec!["first", "second"]);
    }

    #[test]
    fn rotates_provider_keys_in_stable_order() {
        let keys = vec!["one".into(), "two".into()];
        let cursor = AtomicUsize::new(0);

        assert_eq!(AppState::next_key(&keys, &cursor).as_deref(), Some("one"));
        assert_eq!(AppState::next_key(&keys, &cursor).as_deref(), Some("two"));
        assert_eq!(AppState::next_key(&keys, &cursor).as_deref(), Some("one"));
    }

    #[test]
    fn serializes_and_rotates_meta_keys() {
        let json = r#"{"meta":["meta-key-1", "meta-key-2"]}"#;
        let keys: ApiKeys = serde_json::from_str(json).unwrap();
        assert_eq!(keys.meta, vec!["meta-key-1", "meta-key-2"]);

        let cursor = AtomicUsize::new(0);
        assert_eq!(
            AppState::next_key(&keys.meta, &cursor).as_deref(),
            Some("meta-key-1")
        );
        assert_eq!(
            AppState::next_key(&keys.meta, &cursor).as_deref(),
            Some("meta-key-2")
        );
        assert_eq!(
            AppState::next_key(&keys.meta, &cursor).as_deref(),
            Some("meta-key-1")
        );
    }
}

/// Payload received from the frontend when the user changes the active
/// engine and sanitizer model in the settings screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfigPayload {
    pub engine: TranscriptionEngine,
    pub sanitizer: SanitizerModel,
    #[serde(default)]
    pub dual_engine: bool,
    #[serde(default)]
    pub reasoning_enabled: bool,
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
    /// Deepgram transport mode (`batch` | `streaming_final`). Defaults to
    /// batch so older frontends that omit the field keep working.
    #[serde(default)]
    pub deepgram_mode: DeepgramMode,
}

fn default_reasoning_effort() -> String {
    "medium".to_string()
}

/// User-customisable global recording shortcuts. Persisted to
/// `shortcuts.json` in the app data directory and re-registered with the
/// global-shortcut plugin whenever the user rebinds them. Strings use the
/// `global-hotkey` format parsed by the plugin, e.g. `"Control+B"`,
/// `"Control+Shift+Q"`, `"Alt+F2"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutConfig {
    pub toggle: String,
    pub cancel: String,
    /// Voice command: speak an instruction applied to the selected text.
    /// Empty disables it.
    #[serde(default = "default_command_shortcut")]
    pub command: String,
    /// Record only while the shortcut is held down.
    #[serde(default)]
    pub hold_to_talk: bool,
}

fn default_command_shortcut() -> String {
    "Control+Shift+B".to_string()
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            toggle: "Control+B".to_string(),
            cancel: "Control+Q".to_string(),
            command: default_command_shortcut(),
            hold_to_talk: false,
        }
    }
}

/// Text captured from the focused app when a voice command starts.
#[derive(Debug, Clone, Default)]
pub struct CommandRequest {
    pub selection: Option<String>,
}

/// Payload received from the frontend when the user saves API keys
/// in the settings screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeysPayload {
    #[serde(default)]
    pub groq: Vec<String>,
    #[serde(default)]
    pub google: Vec<String>,
    #[serde(default)]
    pub deepgram: Vec<String>,
    #[serde(default)]
    pub openrouter: Vec<String>,
    #[serde(default)]
    pub meta: Vec<String>,
}

/// Global application state guarded by locks for safe concurrent
/// access across Tauri commands and the global shortcut listener.
///
/// Configuration fields use `RwLock` (reads outnumber writes) while
/// the audio pipeline uses `Mutex` because the data callback pushes
/// samples in short, exclusive bursts. The active `cpal::Stream`
/// handle is stored as `Option` so it can be swapped out cleanly when
/// a recording starts or stops.
pub static CONFIG_LOCK: parking_lot::ReentrantMutex<()> = parking_lot::ReentrantMutex::new(());

pub struct AppState {
    pub engine: RwLock<TranscriptionEngine>,
    pub sanitizer: RwLock<SanitizerModel>,
    pub recording: RwLock<bool>,
    recording_lifecycle: Mutex<RecordingLifecycle>,
    pub operations: Arc<crate::operations::Coordinator>,
    pub capture_lease: Mutex<Option<crate::operations::Lease>>,
    pub api_keys: RwLock<ApiKeys>,
    pub audio_stream: Mutex<Option<Stream>>,
    pub audio_buffer: Mutex<Vec<i16>>,
    pub capture_spool: Mutex<Option<crate::capture_spool::CaptureSpool>>,
    pub capture_fault: Mutex<Option<String>>,
    pub test_stream: Mutex<Option<Stream>>,
    pub test_lease: Mutex<Option<crate::operations::Lease>>,
    /// Native sample rate (Hz) of the active capture stream. The microphone is
    /// recorded at the device's own rate and resampled to 16 kHz only when the
    /// final WAV is assembled, because forcing an unsupported rate on the
    /// stream itself fails on Windows/WASAPI shared mode.
    pub capture_rate: RwLock<u32>,
    /// Wall-clock instant the current recording began, used to compute the
    /// elapsed time so the timer survives navigating between views. `None`
    /// while idle.
    pub recording_since: Mutex<Option<std::time::Instant>>,
    pub system_prompt: RwLock<String>,
    /// Legacy mirror kept for compatibility with older IPC clients. New code
    /// uses `widget_visibility_mode` exclusively.
    pub compact_mode: RwLock<bool>,
    pub widget_visibility_mode: RwLock<WidgetVisibilityMode>,
    pub widget_dock: RwLock<WidgetDock>,
    /// Current monitor anchor. The native watcher updates it whenever the
    /// visible gadget's foreground target moves between monitors.
    pub gadget_session_anchor: Mutex<Option<GadgetSessionAnchor>>,
    pub gadget_visual_state: RwLock<GadgetVisualState>,
    /// Monotonic native presentation id and the newest frontend-confirmed id.
    /// They power paint acknowledgement and bounded WebView recovery.
    pub gadget_presentation_generation: AtomicU64,
    pub gadget_rendered_generation: AtomicU64,
    /// User-customisable global recording shortcuts (toggle/cancel).
    pub shortcuts: RwLock<ShortcutConfig>,
    /// Tauri `AppHandle` set once during `setup`. Held by the audio
    /// pipeline so a finished transcription can be persisted to disk and
    /// announced to the UI via the `transcription-saved` event without
    /// needing access to an `AppHandle` injection site.
    pub app_handle: RwLock<Option<tauri::AppHandle>>,
    pub dual_engine: RwLock<bool>,
    pub reasoning_enabled: RwLock<bool>,
    /// Deepgram transport: REST batch vs WebSocket streaming (final only).
    /// Persisted to `settings.json`. Only used when Deepgram is in the path.
    pub deepgram_mode: RwLock<DeepgramMode>,
    /// Live Deepgram WebSocket session (streaming_final + mic). Opened when
    /// recording starts and finished/aborted when recording stops/cancels.
    /// `None` while idle or in batch mode.
    /// When `true` (default), the acoustic transcription is passed through the
    /// Groq Chat Completions sanitizer (Stage 2) for orthographic cleanup and
    /// formatting. When `false`, the **raw** acoustic transcription is copied
    /// to the clipboard / pasted directly, skipping the sanitization round-
    /// trip entirely — useful when the user wants unmodified output or wants
    /// to save the extra network call. Persisted to `settings.json`.
    pub sanitizer_enabled: RwLock<bool>,
    pub reasoning_effort: RwLock<String>,
    /// Structured vocabulary (canonical, aliases, category, strict, enabled).
    /// Persisted to `settings.json`; legacy `custom_words` is migrated on load.
    pub vocabulary: RwLock<Vec<crate::vocabulary::VocabularyTerm>>,
    /// Latest visible-pill rectangle reported by the gadget overlay. `None`
    /// until the gadget reports for the first time. Consumed by the cursor
    /// watcher to make the overlay click-through outside the pill.
    pub gadget_hit_rect: RwLock<Option<GadgetHitRect>>,
    /// Compatibility flag kept for persisted settings and older frontends.
    /// Product pipelines are always active in the current application.
    pub modes_enabled: RwLock<bool>,
    /// Selected product mode (UltraFast / FastAccurate / …).
    pub transcription_mode: RwLock<crate::pipeline_contract::TranscriptionMode>,
    /// FastAccurate: if Gemini fails or is unavailable, fall back to Whisper.
    pub gemini_fallback_to_whisper: RwLock<bool>,
    /// Adds plain `@file.ext` mentions when Gemini recognizes an explicit file reference.
    pub file_tagging_enabled: RwLock<bool>,
    pub gemini_pipelines: RwLock<crate::pipeline_contract::GeminiPipelineConfig>,
    /// Privacy policy and source toggles used to capture a best-effort context
    /// snapshot at the exact start of each recording.
    pub context_preferences: RwLock<crate::context::ContextPreferences>,
    /// Immutable session snapshot consumed by the transcription and delivery
    /// stages after recording stops.
    pub recording_session: Mutex<Option<crate::pipeline_run::RecordingSession>>,
    pub output_profiles: RwLock<Vec<crate::output_policy::OutputProfile>>,
    pub formatting_level: RwLock<crate::output_policy::FormattingLevel>,
    pub dictation_destination: RwLock<crate::output_policy::DictationDestination>,
    pub features: RwLock<crate::assist::FeatureSettings>,
    /// Set when the current recording was started by the voice-command shortcut:
    /// the transcript becomes an instruction applied to this selection.
    pub command_request: Mutex<Option<CommandRequest>>,
    pub temporary_profile_override: RwLock<Option<String>>,
    /// Failure journal handed from provider orchestration to history when a
    /// top-level Result must remain backward-compatible with `String` errors.
    pub pending_failed_pipeline_run: Mutex<Option<crate::pipeline_run::PipelineRun>>,
    groq_key_cursor: Arc<AtomicUsize>,
    google_key_cursor: Arc<AtomicUsize>,
    deepgram_key_cursor: Arc<AtomicUsize>,
    openrouter_key_cursor: Arc<AtomicUsize>,
    meta_key_cursor: Arc<AtomicUsize>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            engine: RwLock::new(TranscriptionEngine::default()),
            sanitizer: RwLock::new(SanitizerModel::default()),
            recording: RwLock::new(false),
            recording_lifecycle: Mutex::new(RecordingLifecycle::default()),
            operations: Arc::new(crate::operations::Coordinator::default()),
            capture_lease: Mutex::new(None),
            api_keys: RwLock::new(ApiKeys::default()),
            audio_stream: Mutex::new(None),
            audio_buffer: Mutex::new(Vec::new()),
            capture_spool: Mutex::new(None),
            capture_fault: Mutex::new(None),
            test_stream: Mutex::new(None),
            test_lease: Mutex::new(None),
            capture_rate: RwLock::new(16_000),
            recording_since: Mutex::new(None),
            system_prompt: RwLock::new(String::new()),
            compact_mode: RwLock::new(false),
            widget_visibility_mode: RwLock::new(WidgetVisibilityMode::Auto),
            widget_dock: RwLock::new(WidgetDock::Bottom),
            gadget_session_anchor: Mutex::new(None),
            gadget_visual_state: RwLock::new(GadgetVisualState::Hidden),
            gadget_presentation_generation: AtomicU64::new(0),
            gadget_rendered_generation: AtomicU64::new(0),
            shortcuts: RwLock::new(ShortcutConfig::default()),
            app_handle: RwLock::new(None),
            dual_engine: RwLock::new(false),
            reasoning_enabled: RwLock::new(false),
            deepgram_mode: RwLock::new(DeepgramMode::default()),
            sanitizer_enabled: RwLock::new(true),
            reasoning_effort: RwLock::new("medium".to_string()),
            vocabulary: RwLock::new(Vec::new()),
            gadget_hit_rect: RwLock::new(None),
            modes_enabled: RwLock::new(true),
            transcription_mode: RwLock::new(crate::pipeline_contract::TranscriptionMode::UltraFast),
            gemini_fallback_to_whisper: RwLock::new(true),
            file_tagging_enabled: RwLock::new(true),
            gemini_pipelines: RwLock::new(crate::pipeline_contract::GeminiPipelineConfig::default()),
            context_preferences: RwLock::new(crate::context::ContextPreferences::default()),
            recording_session: Mutex::new(None),
            output_profiles: RwLock::new(Vec::new()),
            formatting_level: RwLock::new(crate::output_policy::FormattingLevel::default()),
            features: RwLock::new(crate::assist::FeatureSettings::default()),
            command_request: Mutex::new(None),
            dictation_destination: RwLock::new(
                crate::output_policy::DictationDestination::default(),
            ),
            temporary_profile_override: RwLock::new(None),
            pending_failed_pipeline_run: Mutex::new(None),
            groq_key_cursor: Arc::new(AtomicUsize::new(0)),
            google_key_cursor: Arc::new(AtomicUsize::new(0)),
            deepgram_key_cursor: Arc::new(AtomicUsize::new(0)),
            openrouter_key_cursor: Arc::new(AtomicUsize::new(0)),
            meta_key_cursor: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn next_key(keys: &[String], cursor: &AtomicUsize) -> Option<String> {
        if keys.is_empty() {
            return None;
        }
        let index = cursor.fetch_add(1, Ordering::Relaxed) % keys.len();
        Some(keys[index].clone())
    }

    pub fn next_groq_key(&self) -> Option<String> {
        Self::next_key(&self.api_keys.read().groq, &self.groq_key_cursor)
    }

    pub fn next_google_key(&self) -> Option<String> {
        Self::next_key(&self.api_keys.read().google, &self.google_key_cursor)
    }

    pub fn next_openrouter_key(&self) -> Option<String> {
        Self::next_key(
            &self.api_keys.read().openrouter,
            &self.openrouter_key_cursor,
        )
    }

    /// Provider orchestration reads one immutable configuration snapshot per job.
    pub fn pipeline_snapshot(&self) -> Arc<Self> {
        let _config = CONFIG_LOCK.lock();
        let mut snapshot = Self::new();
        snapshot.groq_key_cursor = self.groq_key_cursor.clone();
        snapshot.google_key_cursor = self.google_key_cursor.clone();
        snapshot.deepgram_key_cursor = self.deepgram_key_cursor.clone();
        snapshot.openrouter_key_cursor = self.openrouter_key_cursor.clone();
        snapshot.meta_key_cursor = self.meta_key_cursor.clone();
        snapshot.operations = self.operations.clone();
        *snapshot.engine.write() = *self.engine.read();
        *snapshot.sanitizer.write() = *self.sanitizer.read();
        *snapshot.api_keys.write() = self.api_keys.read().clone();
        *snapshot.system_prompt.write() = self.system_prompt.read().clone();
        *snapshot.app_handle.write() = self.app_handle.read().clone();
        *snapshot.dual_engine.write() = *self.dual_engine.read();
        *snapshot.deepgram_mode.write() = *self.deepgram_mode.read();
        *snapshot.sanitizer_enabled.write() = *self.sanitizer_enabled.read();
        *snapshot.reasoning_enabled.write() = *self.reasoning_enabled.read();
        *snapshot.reasoning_effort.write() = self.reasoning_effort.read().clone();
        *snapshot.vocabulary.write() = self.vocabulary.read().clone();
        *snapshot.modes_enabled.write() = *self.modes_enabled.read();
        *snapshot.transcription_mode.write() = *self.transcription_mode.read();
        *snapshot.gemini_fallback_to_whisper.write() = *self.gemini_fallback_to_whisper.read();
        *snapshot.file_tagging_enabled.write() = *self.file_tagging_enabled.read();
        *snapshot.gemini_pipelines.write() = self.gemini_pipelines.read().clone();
        *snapshot.context_preferences.write() = self.context_preferences.read().clone();
        *snapshot.output_profiles.write() = self.output_profiles.read().clone();
        *snapshot.formatting_level.write() = *self.formatting_level.read();
        *snapshot.dictation_destination.write() = *self.dictation_destination.read();
        *snapshot.features.write() = self.features.read().clone();
        *snapshot.temporary_profile_override.write() =
            self.temporary_profile_override.read().clone();
        *snapshot.recording_session.lock() = self.recording_session.lock().clone();
        Arc::new(snapshot)
    }

    pub fn next_meta_key(&self) -> Option<String> {
        Self::next_key(&self.api_keys.read().meta, &self.meta_key_cursor)
    }

    /// Convenience helper used by the panic shortcut and the toggle
    /// command to flip the recording flag and return the new value.
    pub fn set_recording(&self, value: bool) -> bool {
        let mut guard = self.recording.write();
        *guard = value;
        value
    }

    pub(crate) fn toggle_recording_lifecycle(&self) -> RecordingToggle {
        let mut lifecycle = self.recording_lifecycle.lock();
        match lifecycle.phase {
            RecordingPhase::Idle => {
                let Ok(lease) = self.operations.begin("microphone") else {
                    return RecordingToggle::Busy(lifecycle.snapshot());
                };
                *self.capture_lease.lock() = Some(lease);

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let session_id = format!("recording-session-{}-{}", now, lifecycle.generation + 1);
                let status = lifecycle
                    .begin(session_id)
                    .expect("idle lifecycle must accept a recording start");
                self.set_recording(true);
                RecordingToggle::Start(status)
            }
            RecordingPhase::Starting | RecordingPhase::Recording => {
                let status = lifecycle
                    .request_stop()
                    .expect("active lifecycle must accept a recording stop");
                self.set_recording(false);
                RecordingToggle::Stop(status)
            }
            RecordingPhase::Stopping | RecordingPhase::Cancelling => {
                RecordingToggle::Busy(lifecycle.snapshot())
            }
        }
    }

    pub fn recording_status(&self) -> RecordingStatus {
        self.recording_lifecycle.lock().snapshot()
    }

    pub(crate) fn install_recording_capture(
        &self,
        generation: u64,
        stream: Stream,
        session: crate::pipeline_run::RecordingSession,
    ) -> Result<(), Box<(Stream, crate::pipeline_run::RecordingSession)>> {
        let lifecycle = self.recording_lifecycle.lock();
        if lifecycle.generation != generation || lifecycle.phase != RecordingPhase::Starting {
            return Err(Box::new((stream, session)));
        }
        *self.audio_stream.lock() = Some(stream);
        *self.recording_session.lock() = Some(session);
        Ok(())
    }

    pub(crate) fn recording_capture_ready(
        &self,
        generation: u64,
    ) -> Option<(bool, RecordingStatus)> {
        let result = self.recording_lifecycle.lock().capture_ready(generation);
        if matches!(result, Some((true, _))) {
            self.set_recording(true);
        }
        result
    }

    pub(crate) fn recording_capture_failed(&self, generation: u64) -> Option<RecordingStatus> {
        let status = self.recording_lifecycle.lock().capture_failed(generation);
        self.set_recording(false);
        status
    }

    pub(crate) fn recording_capture_start_pending(&self, generation: u64) -> bool {
        let lifecycle = self.recording_lifecycle.lock();
        lifecycle.generation == generation && lifecycle.capture_start_pending
    }

    pub(crate) fn set_recording_delivery_target(
        &self,
        target: crate::context::ForegroundTarget,
    ) -> bool {
        let mut session = self.recording_session.lock();
        let Some(session) = session.as_mut() else {
            return false;
        };
        session.delivery_target = target;
        true
    }

    pub(crate) fn finish_recording_stop(&self, generation: u64) -> RecordingStatus {
        self.set_recording(false);
        self.recording_lifecycle.lock().finish_stop(generation)
    }

    pub(crate) fn request_recording_cancel(&self) -> RecordingStatus {
        self.set_recording(false);
        self.recording_lifecycle.lock().request_cancel()
    }

    pub(crate) fn finish_recording_cancel(&self, generation: u64) -> RecordingStatus {
        self.set_recording(false);
        self.recording_lifecycle.lock().finish_cancel(generation)
    }

    /// Returns a snapshot of the current recording state.
    pub fn is_recording(&self) -> bool {
        *self.recording.read()
    }

    /// Stamps the moment the current recording began.
    pub fn mark_recording_start(&self) {
        *self.recording_since.lock() = Some(std::time::Instant::now());
    }

    /// Clears the recording start stamp (on stop or cancel).
    pub fn clear_recording_start(&self) {
        *self.recording_since.lock() = None;
    }

    /// Milliseconds elapsed since the current recording began, or `0` if idle.
    /// Lets the UI restore the timer after navigating away and back.
    pub fn recording_elapsed_ms(&self) -> u64 {
        self.recording_since
            .lock()
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0)
    }

    /// Returns a snapshot of the active engine.
    pub fn active_engine(&self) -> TranscriptionEngine {
        *self.engine.read()
    }

    /// Empties the in-memory sample buffer. Called both when a new
    /// recording starts (to discard stale data) and when a recording
    /// is cancelled (to free the allocated capacity).
    pub fn clear_audio_buffer(&self) {
        self.audio_buffer.lock().clear();
    }

    /// Takes ownership of the accumulated samples, leaving an empty
    /// buffer in place. Used when finalising a WAV file so the state
    /// is ready for the next session.
    pub fn drain_audio_buffer(&self) -> Vec<i16> {
        std::mem::take(&mut *self.audio_buffer.lock())
    }

    /// Computes a normalised loudness level (0.0..=1.0) over the most recent
    /// `window` samples currently buffered. Used by the gadget's live waveform:
    /// a background thread polls this while recording and emits the value to the
    /// overlay window. Returns `0.0` when the buffer is empty. The RMS is scaled
    /// up because speech rarely approaches full-scale amplitude.
    pub fn recent_level(&self, window: usize) -> f32 {
        // Copy the small analysis window and release the capture lock before
        // calculating RMS. This keeps the callback's critical section short
        // without forcing it to discard samples when the meter runs.
        let samples = {
            let buf = self.audio_buffer.lock();
            let n = buf.len();
            if n == 0 {
                return 0.0;
            }
            let start = n.saturating_sub(window);
            buf[start..].to_vec()
        };
        let sum_sq: f64 = samples
            .iter()
            .map(|&s| {
                let f = s as f64 / 32768.0;
                f * f
            })
            .sum();
        let rms = (sum_sq / samples.len() as f64).sqrt();
        // Perceptual boost: conversational speech sits far below full-scale,
        // so amplify aggressively for the gadget waveform. Tuned so normal
        // talking near the mic reliably pushes the meter into the upper range.
        ((rms * 7.5) as f32).clamp(0.0, 1.0)
    }

    /// Stops and drops the active input stream (if any), releasing
    /// the microphone hardware immediately. Returns `true` if a
    /// stream was actually dropped.
    pub fn drop_audio_stream(&self) -> bool {
        self.audio_stream.lock().take().is_some()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// cpal::Stream contains Windows COM raw pointers (*mut ()) and is marked NotSendSyncAcrossAllPlatforms.
// Since all fields in AppState are wrapped in Mutex or RwLock, it is safe to implement Send and Sync manually.
unsafe impl Send for AppState {}
unsafe impl Sync for AppState {}

/// Type alias used by Tauri's managed state. Wrapped in Arc so the
/// global shortcut handler can hold a cheap clone of the handle.
pub type SharedState = Arc<AppState>;

/// Names of the events emitted to the frontend. Kept as a small
/// module-level constant list to avoid magic strings drifting between
/// the Rust and TypeScript sides of the codebase.
pub mod event_names {
    pub const RECORDING_INITIALIZING: &str = "recording-initializing";
    pub const RECORDING_STARTED: &str = "recording-started";
    pub const RECORDING_STOPPED: &str = "recording-stopped";
    pub const RECORDING_CANCELLED: &str = "recording-cancelled";
    pub const RECORDING_IDLE: &str = "recording-idle";
    /// Emitted after a transcription has been produced and persisted to
    /// the history file. Payload is the full [`HistoryEntry`] snapshot.
    pub const TRANSCRIPTION_SAVED: &str = "transcription-saved";
    /// Low-volume structured progress for the gadget and Pipeline Inspector.
    pub const PIPELINE_PROGRESS: &str = "pipeline-progress";
    /// Emitted on every poll tick while recording with a normalised loudness
    /// level (`f32` in 0.0..=1.0) so the gadget can animate its live waveform.
    pub const AUDIO_LEVEL: &str = "audio-level";
    /// Emitted when the gadget compact-mode preference changes. Payload is the
    /// new `bool` value.
    pub const COMPACT_MODE_CHANGED: &str = "compact-mode-changed";
    pub const WIDGET_PREFERENCES_CHANGED: &str = "widget-preferences-changed";
}

/// Developer-mode snapshot of the sanitizer (Groq Chat Completions) request and
/// response for a single transcription. Captured on every sanitization so the
/// Histórico can expose exactly what was sent — model, parameters, the reasoning
/// level actually applied and the raw JSON body — when developer mode is on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SanitizerDebug {
    /// Endpoint the request was POSTed to.
    pub endpoint: String,
    /// Exact `model` id sent to Groq (e.g. `openai/gpt-oss-120b`).
    pub model: String,
    /// Decoding temperature sent in the request.
    pub temperature: f32,
    /// Whether the user had reasoning enabled in settings.
    pub reasoning_enabled: bool,
    /// Reasoning effort selected in settings (`low`/`medium`/`high`).
    pub reasoning_effort: String,
    /// Whether `reasoning_effort` was actually sent as a request parameter.
    /// True only when reasoning is enabled *and* the model supports it.
    pub reasoning_effort_applied: bool,
    /// Whether the selected model supports Groq's native reasoning parameter.
    pub reasoning_supported_by_model: bool,
    /// Final system prompt assembled and sent (base + glossary + dual-engine).
    pub system_prompt: String,
    /// User message sent (the raw `[WHISPER_RAW]` / `[DEEPGRAM_RAW]` payload).
    pub user_message: String,
    /// The exact request body serialized to pretty JSON.
    pub request_json: String,
    /// HTTP status of the response, when a response was received.
    #[serde(default)]
    pub response_status: Option<u16>,
    /// Final content returned by the model (the sanitized text).
    #[serde(default)]
    pub response_content: Option<String>,
    /// Reasoning trace returned by the model, truncated for storage.
    #[serde(default)]
    pub response_reasoning: Option<String>,
    /// Error string when the request failed (network/api/parse).
    #[serde(default)]
    pub error: Option<String>,
}

/// A single persisted transcription entry. Serialized to JSON in the
/// app data directory and also sent to the frontend over events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Persisted history schema. Legacy entries deserialize as `0` and are
    /// upgraded in memory without deleting their original projection fields.
    #[serde(default)]
    pub schema_version: u32,
    /// Stable unique id (UTC milliseconds at creation time as a string).
    pub id: String,
    /// Human-readable timestamp, e.g. "2026-06-20 14:32".
    pub date: String,
    /// Number of words in the transcribed text.
    pub words: usize,
    /// Engine that produced the raw transcription.
    pub engine: String,
    /// Final text (after sanitization).
    pub text: String,
    /// Absolute path to the persisted source audio on disk, if it was saved
    /// (every new transcription saves one; older entries created before audio
    /// persistence existed have `None`). Required for pronunciation evaluation.
    #[serde(default)]
    pub audio_path: Option<String>,
    /// Markdown pronunciation feedback returned by Gemini, populated lazily the
    /// first time the user clicks "Avaliar Pronúncia" on this entry.
    #[serde(default)]
    pub evaluation: Option<String>,
    /// Duration of the source audio in milliseconds. Computed from the captured
    /// sample count for microphone recordings; `0` for file uploads (and for
    /// legacy entries created before this field existed).
    #[serde(default)]
    pub duration_ms: u64,
    /// How the transcription was produced: `"mic"` for a microphone recording
    /// or `"file"` for an uploaded audio file. Empty on legacy entries.
    #[serde(default)]
    pub source: String,
    /// Total processing latency in milliseconds.
    #[serde(default)]
    pub latency_ms: u64,
    /// Model generation throughput in tokens per second.
    #[serde(default)]
    pub throughput: f64,
    #[serde(default)]
    pub transcription_latency_ms: Option<u64>,
    #[serde(default)]
    pub sanitizer_latency_ms: Option<u64>,
    #[serde(default)]
    pub transcription_throughput: Option<f64>,
    #[serde(default)]
    pub sanitizer_throughput: Option<f64>,
    /// Real-time factor for the acoustic stage: `transcription_latency_ms / duration_ms`.
    /// Values below 1.0 mean faster than real-time (e.g. 0.25 = 4× realtime).
    /// Useful to compare Deepgram batch vs streaming_final on the same clip.
    #[serde(default)]
    pub realtime_factor: Option<f64>,
    /// Deepgram transport used when Deepgram was in the path (`batch` or
    /// `streaming_final`). `None` when only Groq/Gemini ran, or on legacy
    /// entries created before this field existed.
    #[serde(default)]
    pub deepgram_mode: Option<String>,
    #[serde(default)]
    pub total_tokens: Option<usize>,
    #[serde(default)]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub error_message: Option<String>,
    /// Developer-mode capture of the sanitizer request/response. Populated on
    /// every successful sanitization; `None` for failed transcriptions (the
    /// sanitizer never ran) and for legacy entries created before this field.
    #[serde(default)]
    pub debug_info: Option<SanitizerDebug>,
    /// Product mode id when the new pipeline ran (`ultra-fast`, `fast-accurate`, …).
    #[serde(default)]
    pub mode: Option<String>,
    /// Primary model id used for the run (e.g. whisper-large-v3-turbo).
    #[serde(default)]
    pub model: Option<String>,
    /// Comma-separated stage names executed (e.g. `whisper`, `gemini_transcribe,whisper_fallback`).
    #[serde(default)]
    pub stages: Option<String>,
    /// True when a fallback path produced the final text.
    #[serde(default)]
    pub used_fallback: Option<bool>,
    /// Machine-readable fallback reason when [`used_fallback`] is true.
    #[serde(default)]
    pub fallback_reason: Option<String>,
    /// Content-type hint used for the run (`auto`, `programming`, …).
    #[serde(default)]
    pub content_type: Option<String>,
    /// Intermediate Whisper text when available.
    #[serde(default)]
    pub whisper_text: Option<String>,
    /// Intermediate sanitizer text when available.
    #[serde(default)]
    pub sanitizer_text: Option<String>,
    /// Intermediate Gemini text when available.
    #[serde(default)]
    pub gemini_text: Option<String>,
    /// Pipeline warnings (parse fallback, strict literals, etc.).
    #[serde(default)]
    pub warnings: Option<Vec<String>>,
    /// Structured stage timings (product modes). Absent on legacy entries.
    #[serde(default)]
    pub audio_prepare_ms: Option<u64>,
    #[serde(default)]
    pub base64_ms: Option<u64>,
    #[serde(default)]
    pub whisper_ms: Option<u64>,
    #[serde(default)]
    pub sanitizer_ms: Option<u64>,
    #[serde(default)]
    pub files_upload_ms: Option<u64>,
    #[serde(default)]
    pub files_poll_ms: Option<u64>,
    #[serde(default)]
    pub files_poll_count: Option<u32>,
    #[serde(default)]
    pub gemini_generate_ms: Option<u64>,
    #[serde(default)]
    pub gemini_delete_ms: Option<u64>,
    #[serde(default)]
    pub strict_literals_ms: Option<u64>,
    #[serde(default)]
    pub clipboard_ms: Option<u64>,
    #[serde(default)]
    pub total_pipeline_ms: Option<u64>,
    /// `inline` | `files_api`
    #[serde(default)]
    pub gemini_transport: Option<String>,
    /// Every execution associated with this dictation. Retries and forced
    /// fallback runs append here instead of erasing the prior evidence.
    #[serde(default)]
    pub pipeline_runs: Vec<crate::pipeline_run::PipelineRun>,
}
