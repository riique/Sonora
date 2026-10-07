//! Canonical representation of one pipeline execution.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::context::ContextSnapshot;
use crate::models::SanitizerDebug;
use crate::output_policy::{DictationDestination, FormattingLevel, ResolvedOutputProfile};
use crate::pipeline_contract::{ContentType, TranscriptionMode};

pub const PIPELINE_RUN_SCHEMA_VERSION: u32 = 2;

pub fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptVersions {
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub refined: Option<String>,
    #[serde(default)]
    pub formatted: Option<String>,
    #[serde(default)]
    pub delivered: Option<String>,
    #[serde(default)]
    pub user_corrected: Option<String>,
}

impl TranscriptVersions {
    pub fn current(&self) -> &str {
        self.user_corrected
            .as_deref()
            .or(self.delivered.as_deref())
            .or(self.formatted.as_deref())
            .or(self.refined.as_deref())
            .or(self.raw.as_deref())
            .unwrap_or("")
    }

    pub fn set_raw_once(&mut self, value: impl Into<String>) {
        if self.raw.is_none() {
            self.raw = Some(value.into());
        }
    }

    pub fn set_user_corrected(&mut self, value: impl Into<String>) {
        self.user_corrected = Some(value.into());
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioTransport {
    #[default]
    InlineBase64,
    Multipart,
    RawBinary,
    ResumableFile,
    Url,
    #[serde(rename = "websocket_stream")]
    WebSocketStream,
}

impl AudioTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InlineBase64 => "inline_base64",
            Self::Multipart => "multipart",
            Self::RawBinary => "raw_binary",
            Self::ResumableFile => "resumable_file",
            Self::Url => "url",
            Self::WebSocketStream => "websocket_stream",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportCapabilities {
    #[serde(default)]
    pub inline_base64: bool,
    #[serde(default)]
    pub multipart: bool,
    #[serde(default)]
    pub raw_binary: bool,
    #[serde(default)]
    pub resumable_file: bool,
    #[serde(default)]
    pub url: bool,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub max_multipart_bytes: Option<u64>,
}

impl TransportCapabilities {
    pub fn supports(&self, transport: AudioTransport) -> bool {
        match transport {
            AudioTransport::InlineBase64 => self.inline_base64,
            AudioTransport::Multipart => self.multipart,
            AudioTransport::RawBinary => self.raw_binary,
            AudioTransport::ResumableFile => self.resumable_file,
            AudioTransport::Url => self.url,
            AudioTransport::WebSocketStream => self.streaming,
        }
    }

    pub fn best_supported(&self, ordered: &[AudioTransport], bytes: u64) -> Option<AudioTransport> {
        ordered.iter().copied().find(|transport| {
            if !self.supports(*transport) {
                return false;
            }
            if *transport == AudioTransport::Multipart
                && self.max_multipart_bytes.is_some_and(|limit| bytes > limit)
            {
                return false;
            }
            true
        })
    }
}

pub fn transport_capabilities(provider: &str, endpoint: &str) -> TransportCapabilities {
    match (provider, endpoint) {
        ("openrouter", "audio/transcriptions") => TransportCapabilities {
            multipart: true,
            max_multipart_bytes: Some(25 * 1024 * 1024),
            ..Default::default()
        },
        ("openrouter", "chat/completions") => TransportCapabilities {
            inline_base64: true,
            ..Default::default()
        },
        ("groq", "audio/transcriptions") => TransportCapabilities {
            multipart: true,
            ..Default::default()
        },
        ("deepgram", "listen") => TransportCapabilities {
            raw_binary: true,
            streaming: true,
            ..Default::default()
        },
        ("google-ai-studio", "generateContent") => TransportCapabilities {
            inline_base64: true,
            resumable_file: true,
            ..Default::default()
        },
        _ => TransportCapabilities::default(),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostKind {
    Actual,
    Estimated,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CostRecord {
    #[serde(default)]
    pub kind: CostKind,
    #[serde(default)]
    pub amount_usd: Option<f64>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    #[serde(default)]
    pub audio_seconds: Option<f64>,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub bytes_sent: Option<u64>,
    #[serde(default)]
    pub cost: CostRecord,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

impl UsageRecord {
    pub fn merge(&mut self, other: &Self) {
        self.audio_seconds = self.audio_seconds.or(other.audio_seconds);
        self.input_tokens = sum_options(self.input_tokens, other.input_tokens);
        self.output_tokens = sum_options(self.output_tokens, other.output_tokens);
        self.total_tokens = sum_options(self.total_tokens, other.total_tokens);
        self.bytes_sent = sum_options(self.bytes_sent, other.bytes_sent);
        if other.cost.kind == CostKind::Actual
            || (self.cost.kind == CostKind::Unknown && other.cost.kind == CostKind::Estimated)
        {
            self.cost = other.cost.clone();
        }
        self.metadata.extend(other.metadata.clone());
    }
}

fn sum_options(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (None, None) => None,
        (left, right) => Some(
            left.unwrap_or_default()
                .saturating_add(right.unwrap_or_default()),
        ),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    #[default]
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineErrorKind {
    Timeout,
    Network,
    Authentication,
    Provider,
    InvalidResponse,
    Privacy,
    UnsupportedTransport,
    Clipboard,
    Delivery,
    #[default]
    Internal,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineError {
    #[serde(default)]
    pub kind: PipelineErrorKind,
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AttemptResultMetadata {
    #[serde(default)]
    pub generation_id: Option<String>,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub output_chars: Option<usize>,
    #[serde(default)]
    pub request_sanitized: Option<serde_json::Value>,
    #[serde(default)]
    pub response_sanitized: Option<serde_json::Value>,
    #[serde(default)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderAttempt {
    pub id: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub transport: AudioTransport,
    #[serde(default)]
    pub started_at_ms: u64,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub status: AttemptStatus,
    #[serde(default)]
    pub error: Option<PipelineError>,
    #[serde(default)]
    pub usage: UsageRecord,
    #[serde(default)]
    pub result: AttemptResultMetadata,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    AudioPrepare,
    Recognition,
    Whisper,
    Deepgram,
    SemanticRefinement,
    Sanitizer,
    GeminiAudio,
    Backtrack,
    Formatting,
    SnippetResolution,
    CodeGuard,
    Delivery,
    Clipboard,
    Fallback,
    Cleanup,
    #[default]
    Finalize,
}

impl StageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AudioPrepare => "audio_prepare",
            Self::Recognition => "recognition",
            Self::Whisper => "whisper",
            Self::Deepgram => "deepgram",
            Self::SemanticRefinement => "semantic_refinement",
            Self::Sanitizer => "sanitizer",
            Self::GeminiAudio => "gemini_audio",
            Self::Backtrack => "backtrack",
            Self::Formatting => "formatting",
            Self::SnippetResolution => "snippet_resolution",
            Self::CodeGuard => "code_guard",
            Self::Delivery => "delivery",
            Self::Clipboard => "clipboard",
            Self::Fallback => "fallback",
            Self::Cleanup => "cleanup",
            Self::Finalize => "finalize",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    #[default]
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StageRecord {
    pub id: String,
    pub stage: StageKind,
    #[serde(default)]
    pub started_at_ms: u64,
    #[serde(default)]
    pub finished_at_ms: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub status: StageStatus,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub transport: Option<AudioTransport>,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub error: Option<PipelineError>,
    #[serde(default)]
    pub usage: UsageRecord,
}

impl StageRecord {
    pub fn completed(stage: StageKind, duration_ms: u64) -> Self {
        let now = epoch_ms();
        Self {
            id: format!("{}-{now}", stage.as_str()),
            stage,
            started_at_ms: now.saturating_sub(duration_ms),
            finished_at_ms: Some(now),
            duration_ms: Some(duration_ms),
            status: StageStatus::Success,
            ..Self::default()
        }
    }

    pub fn failed(stage: StageKind, duration_ms: u64, error: PipelineError) -> Self {
        let mut record = Self::completed(stage, duration_ms);
        record.status = StageStatus::Failed;
        record.error = Some(error);
        record
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineWarning {
    pub stage: StageKind,
    pub code: String,
    pub message: String,
}

impl PipelineWarning {
    pub fn new(stage: StageKind, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage,
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PipelineTimings {
    #[serde(default)]
    pub audio_prepare_ms: Option<u64>,
    #[serde(default)]
    pub base64_ms: Option<u64>,
    #[serde(default)]
    pub request_ms: Option<u64>,
    #[serde(default)]
    pub ttfb_ms: Option<u64>,
    #[serde(default)]
    pub provider_ms: Option<u64>,
    #[serde(default)]
    pub whisper_ms: Option<u64>,
    #[serde(default)]
    pub deepgram_ms: Option<u64>,
    #[serde(default)]
    pub sanitizer_ms: Option<u64>,
    #[serde(default)]
    pub gemini_ms: Option<u64>,
    #[serde(default)]
    pub refinement_ms: Option<u64>,
    #[serde(default)]
    pub backtrack_ms: Option<u64>,
    #[serde(default)]
    pub formatting_ms: Option<u64>,
    #[serde(default)]
    pub snippet_ms: Option<u64>,
    #[serde(default)]
    pub code_guard_ms: Option<u64>,
    #[serde(default)]
    pub delivery_ms: Option<u64>,
    #[serde(default)]
    pub clipboard_ms: Option<u64>,
    #[serde(default)]
    pub cleanup_ms: Option<u64>,
    #[serde(default)]
    pub files_upload_ms: Option<u64>,
    #[serde(default)]
    pub files_poll_ms: Option<u64>,
    #[serde(default)]
    pub files_poll_count: Option<u32>,
    #[serde(default)]
    pub gemini_delete_ms: Option<u64>,
    #[serde(default)]
    pub strict_literals_ms: Option<u64>,
    #[serde(default)]
    pub total_ms: u64,
}

impl PipelineTimings {
    pub fn recompute_total(&mut self) {
        self.total_ms = [
            self.audio_prepare_ms,
            self.provider_ms
                .or(self.whisper_ms)
                .or(self.deepgram_ms)
                .or(self.gemini_ms),
            self.refinement_ms.or(self.sanitizer_ms),
            self.backtrack_ms,
            self.formatting_ms,
            self.snippet_ms,
            self.code_guard_ms,
            self.delivery_ms,
            self.cleanup_ms,
        ]
        .into_iter()
        .flatten()
        .sum();
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackRecord {
    #[serde(default)]
    pub used: bool,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub from_provider: Option<String>,
    #[serde(default)]
    pub to_provider: Option<String>,
    #[serde(default)]
    pub forced: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryRecord {
    #[serde(default)]
    pub target_focus_id: Option<u64>,
    #[serde(default)]
    pub destination: DictationDestination,
    #[serde(default)]
    pub target_hwnd: Option<isize>,
    #[serde(default)]
    pub target_process_id: Option<u32>,
    #[serde(default)]
    pub delivered_at_ms: Option<u64>,
    #[serde(default)]
    pub clipboard_ok: bool,
    #[serde(default)]
    pub paste_attempted: bool,
    #[serde(default)]
    pub paste_ok: bool,
    #[serde(default)]
    pub scratchpad_note_id: Option<String>,
    #[serde(default)]
    pub error: Option<PipelineError>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunStatus {
    #[default]
    Running,
    Success,
    Failed,
    Partial,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineProgressKind {
    AudioPreparing,
    #[default]
    Recognizing,
    ProviderFailed,
    FallbackStarted,
    Refining,
    Formatting,
    Delivering,
    Complete,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineProgressEvent {
    pub kind: PipelineProgressKind,
    #[serde(default)]
    pub operation_id: u64,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub fallback_provider: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

pub fn emit_pipeline_progress(state: &crate::models::AppState, mut event: PipelineProgressEvent) {
    let operation = state.operations.status();
    if operation.as_ref().is_some_and(|job| job.cancelled) {
        return;
    }
    event.operation_id = operation.as_ref().map_or(0, |job| job.id);
    let Some(handle) = state.app_handle.read().as_ref().cloned() else {
        return;
    };
    if operation
        .as_ref()
        .is_some_and(|job| matches!(job.kind.as_str(), "microphone" | "retry-mic"))
    {
        if let Some(gadget) = handle.get_webview_window("gadget") {
            let _ = gadget.emit(crate::models::event_names::PIPELINE_PROGRESS, &event);
        }
    }
    if let Some(main) = handle.get_webview_window("main") {
        let _ = main.emit(crate::models::event_names::PIPELINE_PROGRESS, event);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecordingSession {
    pub id: String,
    #[serde(default)]
    pub started_at_ms: u64,
    #[serde(default)]
    pub context: ContextSnapshot,
    #[serde(default)]
    pub profile: ResolvedOutputProfile,
    #[serde(default)]
    pub formatting_level: FormattingLevel,
    #[serde(default)]
    pub destination: DictationDestination,
    /// Window selected for final delivery. Initialized from the start context
    /// and refreshed from the foreground window at the exact stop shortcut.
    #[serde(default)]
    pub delivery_target: crate::context::ForegroundTarget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineRun {
    #[serde(default = "pipeline_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub session_id: String,
    #[serde(default)]
    pub started_at_ms: u64,
    #[serde(default)]
    pub finished_at_ms: Option<u64>,
    #[serde(default)]
    pub status: PipelineRunStatus,
    pub mode: TranscriptionMode,
    #[serde(default, rename = "content_type")]
    pub content_hint: ContentType,
    #[serde(default)]
    pub context: ContextSnapshot,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub formatting_level: FormattingLevel,
    #[serde(default)]
    pub destination: DictationDestination,
    #[serde(default)]
    pub attempts: Vec<ProviderAttempt>,
    #[serde(default, rename = "stages")]
    pub journal: Vec<StageRecord>,
    #[serde(default)]
    pub transcript: TranscriptVersions,
    #[serde(default)]
    pub delivery: DeliveryRecord,
    #[serde(default)]
    pub fallback: FallbackRecord,
    #[serde(default)]
    pub usage: UsageRecord,
    #[serde(default)]
    pub timings: PipelineTimings,
    #[serde(default, rename = "warnings")]
    pub journal_warnings: Vec<PipelineWarning>,
    #[serde(default)]
    pub error: Option<PipelineError>,
    #[serde(default)]
    pub debug_info: Option<SanitizerDebug>,
    #[serde(default)]
    pub history_engine_label: String,
    // Transient builder compatibility. Product modes write these fields while
    // running; `normalize()` folds them into the canonical nested records
    // before persistence or IPC serialization.
    #[serde(skip)]
    pub final_text: String,
    #[serde(skip)]
    pub model: String,
    #[serde(skip)]
    pub stages: Vec<String>,
    #[serde(skip)]
    pub used_fallback: bool,
    #[serde(skip)]
    pub fallback_reason: Option<String>,
    #[serde(skip)]
    pub whisper_text: Option<String>,
    #[serde(skip)]
    pub deepgram_text: Option<String>,
    #[serde(skip)]
    pub gemini_text: Option<String>,
    #[serde(skip)]
    pub sanitizer_text: Option<String>,
    #[serde(skip)]
    pub transcription_latency_ms: u64,
    #[serde(skip)]
    pub whisper_ms: Option<u64>,
    #[serde(skip)]
    pub upload_ms: Option<u64>,
    #[serde(skip)]
    pub gemini_ms: Option<u64>,
    #[serde(skip)]
    pub audio_prepare_ms: Option<u64>,
    #[serde(skip)]
    pub base64_ms: Option<u64>,
    #[serde(skip)]
    pub sanitizer_ms: Option<u64>,
    #[serde(skip)]
    pub files_upload_ms: Option<u64>,
    #[serde(skip)]
    pub files_poll_ms: Option<u64>,
    #[serde(skip)]
    pub files_poll_count: Option<u32>,
    #[serde(skip)]
    pub gemini_generate_ms: Option<u64>,
    #[serde(skip)]
    pub gemini_delete_ms: Option<u64>,
    #[serde(skip)]
    pub strict_literals_ms: Option<u64>,
    #[serde(skip)]
    pub total_pipeline_ms: Option<u64>,
    #[serde(skip)]
    pub gemini_transport: Option<String>,
    #[serde(skip)]
    pub warnings: Vec<String>,
    #[serde(skip)]
    pub content_type: Option<String>,
    #[serde(skip)]
    pub openrouter_generation_id: Option<String>,
    #[serde(skip)]
    pub reported_total_tokens: Option<usize>,
    #[serde(skip)]
    pub models_used: Vec<String>,
    #[serde(skip)]
    pub is_error: bool,
    #[serde(skip)]
    pub error_message: Option<String>,
}

fn pipeline_schema_version() -> u32 {
    PIPELINE_RUN_SCHEMA_VERSION
}

impl Default for PipelineRun {
    fn default() -> Self {
        Self {
            schema_version: PIPELINE_RUN_SCHEMA_VERSION,
            id: String::new(),
            session_id: String::new(),
            started_at_ms: 0,
            finished_at_ms: None,
            status: PipelineRunStatus::Running,
            mode: TranscriptionMode::default(),
            content_hint: ContentType::default(),
            context: ContextSnapshot::default(),
            profile_id: None,
            formatting_level: FormattingLevel::default(),
            destination: DictationDestination::default(),
            attempts: Vec::new(),
            journal: Vec::new(),
            transcript: TranscriptVersions::default(),
            delivery: DeliveryRecord::default(),
            fallback: FallbackRecord::default(),
            usage: UsageRecord::default(),
            timings: PipelineTimings::default(),
            journal_warnings: Vec::new(),
            error: None,
            debug_info: None,
            history_engine_label: String::new(),
            final_text: String::new(),
            model: String::new(),
            stages: Vec::new(),
            used_fallback: false,
            fallback_reason: None,
            whisper_text: None,
            deepgram_text: None,
            gemini_text: None,
            sanitizer_text: None,
            transcription_latency_ms: 0,
            whisper_ms: None,
            upload_ms: None,
            gemini_ms: None,
            audio_prepare_ms: None,
            base64_ms: None,
            sanitizer_ms: None,
            files_upload_ms: None,
            files_poll_ms: None,
            files_poll_count: None,
            gemini_generate_ms: None,
            gemini_delete_ms: None,
            strict_literals_ms: None,
            total_pipeline_ms: None,
            gemini_transport: None,
            warnings: Vec::new(),
            content_type: None,
            openrouter_generation_id: None,
            reported_total_tokens: None,
            models_used: Vec::new(),
            is_error: false,
            error_message: None,
        }
    }
}

impl PipelineRun {
    pub fn new(id: impl Into<String>, session: &RecordingSession, mode: TranscriptionMode) -> Self {
        Self {
            id: id.into(),
            session_id: session.id.clone(),
            started_at_ms: epoch_ms(),
            mode,
            content_hint: session
                .profile
                .content_type
                .as_deref()
                .and_then(ContentType::parse_str)
                .unwrap_or_default(),
            context: session.context.persisted_metadata(),
            profile_id: Some(session.profile.profile_id.clone()),
            formatting_level: session.formatting_level,
            destination: session.destination,
            delivery: DeliveryRecord {
                destination: session.destination,
                target_hwnd: session.delivery_target.hwnd,
                target_process_id: session.delivery_target.process_id,
                target_focus_id: session.delivery_target.focus_id,
                ..DeliveryRecord::default()
            },
            ..Self::default()
        }
    }

    pub fn success(
        id: impl Into<String>,
        mode: TranscriptionMode,
        final_text: impl Into<String>,
    ) -> Self {
        let final_text = final_text.into();
        let mut run = Self {
            id: id.into(),
            session_id: String::new(),
            mode,
            ..Self::default()
        };
        run.final_text = final_text.clone();
        run.transcript.set_raw_once(final_text.clone());
        run.transcript.refined = Some(final_text.clone());
        run.transcript.formatted = Some(final_text.clone());
        run.transcript.delivered = Some(final_text);
        run.finish_success();
        run
    }

    pub fn hard_error(
        id: impl Into<String>,
        mode: TranscriptionMode,
        message: impl Into<String>,
    ) -> Self {
        let message = message.into();
        let mut run = Self {
            id: id.into(),
            session_id: String::new(),
            mode,
            ..Self::default()
        };
        run.status = PipelineRunStatus::Failed;
        run.finished_at_ms = Some(epoch_ms());
        run.error = Some(PipelineError {
            kind: PipelineErrorKind::Internal,
            code: "pipeline_failed".into(),
            message,
            retryable: true,
        });
        run
    }

    pub fn finish_success(&mut self) {
        self.status = if self.fallback.used {
            PipelineRunStatus::Partial
        } else {
            PipelineRunStatus::Success
        };
        self.finished_at_ms = Some(epoch_ms());
        if self.timings.total_ms == 0 {
            self.timings.recompute_total();
        }
    }

    pub fn canonical_text(&self) -> &str {
        if self.transcript.current().is_empty() {
            &self.final_text
        } else {
            self.transcript.current()
        }
    }

    pub fn add_attempt(&mut self, attempt: ProviderAttempt) {
        self.usage.merge(&attempt.usage);
        self.attempts.push(attempt);
    }

    pub fn add_stage(&mut self, stage: StageRecord) {
        self.usage.merge(&stage.usage);
        self.journal.push(stage);
    }

    pub fn model(&self) -> Option<&str> {
        self.attempts
            .iter()
            .rev()
            .find(|attempt| attempt.status == AttemptStatus::Success)
            .map(|attempt| attempt.model.as_str())
    }

    pub fn normalize(&mut self) {
        self.schema_version = PIPELINE_RUN_SCHEMA_VERSION;
        let now = epoch_ms();
        if self.id.is_empty() {
            self.id = format!("pipeline-run-{now}");
        }
        if self.session_id.is_empty() {
            self.session_id = format!("{}-session", self.id);
        }
        if self.started_at_ms == 0 {
            self.started_at_ms = now.saturating_sub(self.transcription_latency_ms);
        }
        if self.model.is_empty() {
            self.model = self.models_used.last().cloned().unwrap_or_default();
        }
        if let Some(content_type) = self
            .content_type
            .as_deref()
            .and_then(ContentType::parse_str)
        {
            self.content_hint = content_type;
        }
        let raw_candidate = self
            .whisper_text
            .clone()
            .or_else(|| self.deepgram_text.clone())
            .or_else(|| self.gemini_text.clone())
            .unwrap_or_else(|| self.final_text.clone());
        if !raw_candidate.is_empty() {
            self.transcript.set_raw_once(raw_candidate);
        }
        if self.transcript.refined.is_none() {
            self.transcript.refined = self
                .sanitizer_text
                .clone()
                .or_else(|| self.gemini_text.clone())
                .or_else(|| self.transcript.raw.clone());
        }
        if self.transcript.formatted.is_none() {
            self.transcript.formatted = self.transcript.refined.clone();
        }
        if self.transcript.delivered.is_none()
            && !self.final_text.is_empty()
            && self.delivery.error.is_none()
        {
            self.transcript.delivered = Some(self.final_text.clone());
        }

        self.fallback.used |= self.used_fallback;
        self.fallback.reason = self
            .fallback
            .reason
            .clone()
            .or_else(|| self.fallback_reason.clone());

        self.timings.audio_prepare_ms = self.timings.audio_prepare_ms.or(self.audio_prepare_ms);
        self.timings.base64_ms = self.timings.base64_ms.or(self.base64_ms);
        self.timings.provider_ms = self
            .timings
            .provider_ms
            .or(Some(self.transcription_latency_ms).filter(|value| *value > 0));
        self.timings.whisper_ms = self.timings.whisper_ms.or(self.whisper_ms);
        self.timings.sanitizer_ms = self.timings.sanitizer_ms.or(self.sanitizer_ms);
        self.timings.gemini_ms = self
            .timings
            .gemini_ms
            .or(self.gemini_ms)
            .or(self.gemini_generate_ms);
        self.timings.refinement_ms = self.timings.refinement_ms.or(self.sanitizer_ms);
        self.timings.files_upload_ms = self
            .timings
            .files_upload_ms
            .or(self.files_upload_ms)
            .or(self.upload_ms);
        self.timings.files_poll_ms = self.timings.files_poll_ms.or(self.files_poll_ms);
        self.timings.files_poll_count = self.timings.files_poll_count.or(self.files_poll_count);
        self.timings.gemini_delete_ms = self.timings.gemini_delete_ms.or(self.gemini_delete_ms);
        self.timings.strict_literals_ms =
            self.timings.strict_literals_ms.or(self.strict_literals_ms);
        self.timings.total_ms = self.total_pipeline_ms.unwrap_or(self.timings.total_ms);
        if self.timings.total_ms == 0 {
            self.timings.recompute_total();
        }

        if self.usage.total_tokens.is_none() {
            self.usage.total_tokens = self.reported_total_tokens.map(|value| value as u64);
        }

        if self.journal.is_empty() {
            self.journal = self
                .stages
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    let stage = stage_from_legacy_name(name);
                    let mut record =
                        StageRecord::completed(stage, stage_duration(stage, &self.timings));
                    record.id = format!("{}-{index}", stage.as_str());
                    record.metadata.insert(
                        "legacy_name".into(),
                        serde_json::Value::String(name.clone()),
                    );
                    record
                })
                .collect();
        }
        if self.journal_warnings.is_empty() {
            self.journal_warnings = self
                .warnings
                .iter()
                .map(|warning| {
                    PipelineWarning::new(StageKind::Finalize, "pipeline_warning", warning.clone())
                })
                .collect();
        }

        if self.attempts.is_empty() && !self.model.is_empty() {
            let provider = infer_provider(&self.model, &self.stages);
            let transport = self
                .gemini_transport
                .as_deref()
                .and_then(parse_transport)
                .unwrap_or(AudioTransport::InlineBase64);
            self.add_attempt(ProviderAttempt {
                id: format!("{}-attempt-1", self.id),
                provider,
                model: self.model.clone(),
                transport,
                started_at_ms: self.started_at_ms,
                duration_ms: Some(self.transcription_latency_ms),
                status: AttemptStatus::Success,
                usage: UsageRecord {
                    total_tokens: self.reported_total_tokens.map(|value| value as u64),
                    bytes_sent: None,
                    cost: CostRecord::default(),
                    ..UsageRecord::default()
                },
                result: AttemptResultMetadata {
                    generation_id: self.openrouter_generation_id.clone(),
                    output_chars: Some(self.final_text.len()),
                    ..AttemptResultMetadata::default()
                },
                ..ProviderAttempt::default()
            });
        }
        if self.is_error || self.error_message.is_some() {
            self.status = PipelineRunStatus::Failed;
            if self.error.is_none() {
                self.error = Some(PipelineError {
                    kind: PipelineErrorKind::Provider,
                    code: "pipeline_failed".into(),
                    message: self
                        .error_message
                        .clone()
                        .unwrap_or_else(|| "pipeline failed".into()),
                    retryable: true,
                });
            }
        } else if self.finished_at_ms.is_none() {
            self.finish_success();
        }
        self.sum_attempt_costs();
    }

    /// A run can call several paid models (STT, quick refine, voice command).
    /// `UsageRecord::merge` keeps a single cost, so the run total is the sum of
    /// the per-attempt costs whenever any attempt reported one.
    fn sum_attempt_costs(&mut self) {
        let known = self
            .attempts
            .iter()
            .filter(|attempt| attempt.usage.cost.kind != CostKind::Unknown)
            .filter_map(|attempt| {
                attempt
                    .usage
                    .cost
                    .amount_usd
                    .map(|amount| (attempt, amount))
            })
            .collect::<Vec<_>>();
        if known.is_empty() {
            return;
        }
        let all_actual = known
            .iter()
            .all(|(attempt, _)| attempt.usage.cost.kind == CostKind::Actual);
        self.usage.cost = CostRecord {
            kind: if all_actual {
                CostKind::Actual
            } else {
                CostKind::Estimated
            },
            amount_usd: Some(known.iter().map(|(_, amount)| amount).sum()),
            source: Some("sum_of_attempts".into()),
        };
    }
}

fn stage_from_legacy_name(name: &str) -> StageKind {
    let lower = name.to_ascii_lowercase();
    if lower.contains("whisper") || lower.contains("openrouter_stt") {
        StageKind::Whisper
    } else if lower.contains("deepgram") {
        StageKind::Deepgram
    } else if lower.contains("sanitize") {
        StageKind::Sanitizer
    } else if lower.contains("gemini") || lower.contains("files_api") {
        StageKind::GeminiAudio
    } else if lower.contains("fallback") {
        StageKind::Fallback
    } else {
        StageKind::Recognition
    }
}

fn stage_duration(stage: StageKind, timings: &PipelineTimings) -> u64 {
    match stage {
        StageKind::AudioPrepare => timings.audio_prepare_ms,
        StageKind::Whisper => timings.whisper_ms,
        StageKind::Deepgram => timings.deepgram_ms,
        StageKind::Sanitizer | StageKind::SemanticRefinement => {
            timings.sanitizer_ms.or(timings.refinement_ms)
        }
        StageKind::GeminiAudio => timings.gemini_ms,
        StageKind::Backtrack => timings.backtrack_ms,
        StageKind::Formatting => timings.formatting_ms,
        StageKind::SnippetResolution => timings.snippet_ms,
        StageKind::CodeGuard => timings.code_guard_ms,
        StageKind::Delivery => timings.delivery_ms,
        StageKind::Clipboard => timings.clipboard_ms,
        StageKind::Cleanup => timings.cleanup_ms,
        _ => timings.provider_ms,
    }
    .unwrap_or_default()
}

fn infer_provider(model: &str, stages: &[String]) -> String {
    let joined = stages.join(" ").to_ascii_lowercase();
    if joined.contains("openrouter") || model.contains('/') {
        "openrouter".into()
    } else if joined.contains("deepgram") || model.contains("nova") {
        "deepgram".into()
    } else if joined.contains("groq") || model.contains("whisper") {
        "groq".into()
    } else if joined.contains("gemini") || model.contains("gemini") {
        "google-ai-studio".into()
    } else {
        "unknown".into()
    }
}

fn parse_transport(value: &str) -> Option<AudioTransport> {
    match value {
        "inline" | "inline_base64" => Some(AudioTransport::InlineBase64),
        "multipart" => Some(AudioTransport::Multipart),
        "files_api" | "resumable_file" => Some(AudioTransport::ResumableFile),
        "raw_binary" => Some(AudioTransport::RawBinary),
        "url" => Some(AudioTransport::Url),
        "websocket_stream" | "streaming_final" => Some(AudioTransport::WebSocketStream),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_is_immutable_and_user_correction_is_a_new_version() {
        let mut versions = TranscriptVersions::default();
        versions.set_raw_once("open router");
        versions.set_raw_once("should not replace");
        versions.refined = Some("OpenRouter".into());
        versions.set_user_corrected("OpenRouter API");
        assert_eq!(versions.raw.as_deref(), Some("open router"));
        assert_eq!(versions.user_corrected.as_deref(), Some("OpenRouter API"));
    }

    #[test]
    fn actual_cost_wins_over_estimate() {
        let mut aggregate = UsageRecord {
            cost: CostRecord {
                kind: CostKind::Estimated,
                amount_usd: Some(1.0),
                source: Some("table".into()),
            },
            ..UsageRecord::default()
        };
        aggregate.merge(&UsageRecord {
            cost: CostRecord {
                kind: CostKind::Actual,
                amount_usd: Some(0.25),
                source: Some("provider".into()),
            },
            ..UsageRecord::default()
        });
        assert_eq!(aggregate.cost.kind, CostKind::Actual);
        assert_eq!(aggregate.cost.amount_usd, Some(0.25));
    }

    #[test]
    fn transport_selection_never_uses_an_unsupported_mode() {
        let stt = transport_capabilities("openrouter", "audio/transcriptions");
        assert_eq!(
            stt.best_supported(
                &[
                    AudioTransport::RawBinary,
                    AudioTransport::Multipart,
                    AudioTransport::InlineBase64
                ],
                1024,
            ),
            Some(AudioTransport::Multipart)
        );
        assert_eq!(
            stt.best_supported(&[AudioTransport::Multipart], 26 * 1024 * 1024),
            None
        );
        let chat = transport_capabilities("openrouter", "chat/completions");
        assert_eq!(
            chat.best_supported(
                &[AudioTransport::Multipart, AudioTransport::InlineBase64],
                1024
            ),
            Some(AudioTransport::InlineBase64)
        );
    }

    #[test]
    fn failed_primary_and_successful_fallback_remain_separate_attempts() {
        let run = PipelineRun {
            attempts: vec![
                ProviderAttempt {
                    id: "a1".into(),
                    provider: "groq".into(),
                    model: "model-a".into(),
                    status: AttemptStatus::Failed,
                    error: Some(PipelineError {
                        kind: PipelineErrorKind::Timeout,
                        code: "timeout".into(),
                        message: "timeout".into(),
                        retryable: true,
                    }),
                    ..Default::default()
                },
                ProviderAttempt {
                    id: "a2".into(),
                    provider: "deepgram".into(),
                    model: "nova-3".into(),
                    status: AttemptStatus::Success,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(run.attempts.len(), 2);
        assert_eq!(run.attempts[0].status, AttemptStatus::Failed);
        assert_eq!(run.attempts[1].status, AttemptStatus::Success);
    }

    #[test]
    fn stop_time_delivery_target_is_carried_into_the_pipeline_run() {
        let session = RecordingSession {
            id: "session-across-monitors".into(),
            delivery_target: crate::context::ForegroundTarget {
                hwnd: Some(4242),
                process_id: Some(77),
                focus_id: Some(1),
            },
            ..RecordingSession::default()
        };
        let run = PipelineRun::new(
            "run-across-monitors",
            &session,
            TranscriptionMode::UltraFast,
        );
        assert_eq!(run.delivery.target_hwnd, Some(4242));
        assert_eq!(run.delivery.target_process_id, Some(77));
    }
}
