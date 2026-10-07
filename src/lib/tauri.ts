import { invoke } from "@tauri-apps/api/core";
import { normalizeAudioBytes, type BinaryIpcResponse } from "./audio-bytes";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Type-safe wrappers around the Rust IPC commands exposed by the Tauri
 * backend (see `src-tauri/src/commands.rs`).
 *
 * Every function returns a Promise that rejects with the serialized
 * `CommandError` string if the backend returns `Err`.
 */

export type TranscriptionEngine = "groq-whisper" | "deepgram-nova3" | "gemini-multimodal";
export type SanitizerModel =
  | "llama-70b"
  | "gpt-oss-20b"
  | "gpt-oss-120b"
  | "qwen3-27b";

/** Deepgram transport: REST batch vs WebSocket streaming (final only). */
export type DeepgramMode = "batch" | "streaming_final";

/** Product transcription modes (Phase 04+). */
export type TranscriptionMode =
  | "ultra-fast"
  | "fast-accurate"
  | "precise"
  | "ultra-precise";

export interface EngineConfigSnapshot {
  engine: TranscriptionEngine;
  sanitizer: string;
  dual_engine: boolean;
  reasoning_enabled: boolean;
  reasoning_effort: string;
  deepgram_mode: DeepgramMode;
}

export type GeminiModel = "flash-lite35" | "flash36" | "transcribe35" | "muse-voice-transcribe-1.0";
export type GeminiProvider = "google-ai-studio" | "open-router" | "meta";
export type OpenRouterWhisperModel = "large-v3-turbo" | "large-v3";
export interface GeminiPipelineChoice {
  model: GeminiModel;
  provider: GeminiProvider;
  use_custom_model: boolean;
  custom_model: string;
  meta_languages?: string[];
}
export interface GeminiPipelineConfig {
  ultra_fast_whisper: OpenRouterWhisperModel;
  fast_accurate: GeminiPipelineChoice;
  precise: GeminiPipelineChoice;
  ultra_precise: GeminiPipelineChoice;
}

export interface ModeConfigPayload {
  modes_enabled: boolean;
  mode: TranscriptionMode;
  gemini_fallback_to_whisper: boolean;
  file_tagging_enabled: boolean;
  gemini_pipelines: GeminiPipelineConfig;
}

export interface ModeConfigSnapshot {
  modes_enabled: boolean;
  mode: TranscriptionMode;
  gemini_fallback_to_whisper: boolean;
  file_tagging_enabled: boolean;
  gemini_pipelines: GeminiPipelineConfig;
  mode_label: string;
  mode_description: string;
}

export async function getModeConfig(): Promise<ModeConfigSnapshot> {
  return invoke<ModeConfigSnapshot>("get_mode_config");
}

export async function updateModeConfig(
  payload: ModeConfigPayload,
): Promise<ModeConfigSnapshot> {
  return invoke<ModeConfigSnapshot>("update_mode_config", { payload });
}

export interface ApiKeysPayload {
  groq: string[];
  google: string[];
  deepgram: string[];
  openrouter: string[];
  meta: string[];
}

export async function saveApiKeys(payload: ApiKeysPayload): Promise<void> {
  await invoke<void>("save_api_keys", { payload });
}

/** Returns the persisted API keys so the settings screen can prefill them. */
export async function getApiKeys(): Promise<ApiKeysPayload> {
  return invoke<ApiKeysPayload>("get_api_keys");
}

/**
 * Transcribes a local audio file at `path` (selected or dropped in the
 * Transcrição view). Resolves with the final sanitised text and rejects with
 * a readable error string from the backend.
 */
export async function transcribeFile(path: string): Promise<string> {
  return invoke<string>("transcribe_file", { path });
}

/**
 * Sends the saved audio + transcript of history entry `id` to Gemini and
 * resolves with the Markdown pronunciation feedback (also persisted on the
 * entry by the backend).
 */
export async function evaluatePronunciation(id: string): Promise<string> {
  return invoke<string>("evaluate_pronunciation", { id });
}

export interface ShortcutConfig {
  toggle: string;
  cancel: string;
}

/** Returns the currently active recording shortcuts. */
export async function getShortcuts(): Promise<ShortcutConfig> {
  return invoke<ShortcutConfig>("get_shortcuts");
}

/**
 * Rebinds the global start/cancel recording shortcuts. Resolves with the
 * applied config or rejects with a readable error if the combination is
 * invalid or already in use by another application.
 */
export async function setShortcuts(
  toggle: string,
  cancel: string,
): Promise<ShortcutConfig> {
  return invoke<ShortcutConfig>("set_shortcuts", { toggle, cancel });
}

/** Toggles the recording flag in the backend and returns the new state. */
export async function toggleRecordingState(): Promise<boolean> {
  return invoke<boolean>("toggle_recording_state");
}

/** Discards the active capture through the same path as the global cancel shortcut. */
export async function cancelRecording(): Promise<void> {
  await invoke<void>("cancel_recording");
}

export type RecordingPhase =
  | "idle"
  | "starting"
  | "recording"
  | "stopping"
  | "cancelling";

export interface RecordingStatus {
  generation: number;
  revision: number;
  session_id?: string | null;
  phase: RecordingPhase;
  recording: boolean;
  busy: boolean;
}

/** Milliseconds since the active recording started (0 when idle). */
export async function getRecordingElapsed(): Promise<number> {
  return invoke<number>("get_recording_elapsed");
}

/** Monotonic recording lifecycle snapshot from the backend. */
export async function getRecordingStatus(): Promise<RecordingStatus> {
  return invoke<RecordingStatus>("get_recording_status");
}

/** Returns the currently active transcription engine and sanitizer config. */
export async function getEngineConfig(): Promise<EngineConfigSnapshot> {
  return invoke<EngineConfigSnapshot>("get_engine_config");
}

/**
 * Developer-mode capture of the sanitizer (Groq Chat Completions) request and
 * response, mirroring the Rust `SanitizerDebug`. Present on entries transcribed
 * after this feature shipped; inspected from the Histórico when dev mode is on.
 */
export interface SanitizerDebug {
  endpoint: string;
  model: string;
  temperature: number;
  reasoning_enabled: boolean;
  reasoning_effort: string;
  reasoning_effort_applied: boolean;
  reasoning_supported_by_model: boolean;
  system_prompt: string;
  user_message: string;
  request_json: string;
  response_status?: number | null;
  response_content?: string | null;
  response_reasoning?: string | null;
  error?: string | null;
}

export type AudioTransport =
  | "inline_base64"
  | "multipart"
  | "raw_binary"
  | "resumable_file"
  | "url"
  | "websocket_stream";

export interface CostRecord {
  kind: "actual" | "estimated" | "unknown";
  amount_usd?: number | null;
  source?: string | null;
}

export interface UsageRecord {
  audio_seconds?: number | null;
  input_tokens?: number | null;
  output_tokens?: number | null;
  total_tokens?: number | null;
  bytes_sent?: number | null;
  cost: CostRecord;
  metadata?: Record<string, unknown>;
}

export interface PipelineError {
  kind: string;
  code: string;
  message: string;
  retryable?: boolean;
}

export interface ProviderAttempt {
  id: string;
  provider: string;
  model: string;
  transport: AudioTransport;
  started_at_ms?: number;
  duration_ms?: number | null;
  status: "pending" | "running" | "success" | "failed" | "skipped";
  error?: PipelineError | null;
  usage: UsageRecord;
  result: {
    generation_id?: string | null;
    finish_reason?: string | null;
    language?: string | null;
    output_chars?: number | null;
    request_sanitized?: unknown;
    response_sanitized?: unknown;
    extra?: Record<string, unknown>;
  };
}

export interface StageRecord {
  id: string;
  stage: string;
  started_at_ms?: number;
  finished_at_ms?: number | null;
  duration_ms?: number | null;
  status: "pending" | "running" | "success" | "failed" | "skipped";
  provider?: string | null;
  model?: string | null;
  transport?: AudioTransport | null;
  metadata?: Record<string, unknown>;
  error?: PipelineError | null;
  usage: UsageRecord;
}

export interface TranscriptVersions {
  raw?: string | null;
  refined?: string | null;
  formatted?: string | null;
  delivered?: string | null;
  user_corrected?: string | null;
}

export interface PipelineRun {
  schema_version: number;
  id: string;
  session_id: string;
  started_at_ms?: number;
  finished_at_ms?: number | null;
  status: "running" | "success" | "failed" | "partial";
  mode: TranscriptionMode;
  content_type: string;
  context?: Record<string, unknown>;
  profile_id?: string | null;
  formatting_level: "literal" | "smart" | "aggressive";
  destination: "focused_field" | "clipboard_only" | "scratchpad";
  attempts: ProviderAttempt[];
  stages: StageRecord[];
  transcript: TranscriptVersions;
  delivery: Record<string, unknown>;
  fallback: {
    used: boolean;
    reason?: string | null;
    from_provider?: string | null;
    to_provider?: string | null;
    forced?: boolean;
  };
  usage: UsageRecord;
  timings: Record<string, number | null | undefined> & { total_ms: number };
  warnings: Array<{ stage: string; code: string; message: string }>;
  error?: PipelineError | null;
  debug_info?: SanitizerDebug | null;
  history_engine_label?: string;
}

/** A single persisted transcription, mirroring the Rust `HistoryEntry`. */
export interface HistoryEntry {
  schema_version?: number;
  id: string;
  date: string;
  words: number;
  engine: string;
  text: string;
  audio_path?: string | null;
  evaluation?: string | null;
  duration_ms?: number;
  source?: string;
  latency_ms?: number;
  throughput?: number;
  transcription_latency_ms?: number | null;
  sanitizer_latency_ms?: number | null;
  transcription_throughput?: number | null;
  sanitizer_throughput?: number | null;
  /** Acoustic RTF: transcription_latency_ms / duration_ms (< 1 = faster than realtime). */
  realtime_factor?: number | null;
  /** Deepgram transport when used: `batch` | `streaming_final`. */
  deepgram_mode?: string | null;
  total_tokens?: number | null;
  is_error?: boolean | null;
  error_message?: string | null;
  debug_info?: SanitizerDebug | null;
  mode?: string | null;
  model?: string | null;
  stages?: string | null;
  used_fallback?: boolean | null;
  fallback_reason?: string | null;
  content_type?: string | null;
  whisper_text?: string | null;
  sanitizer_text?: string | null;
  gemini_text?: string | null;
  warnings?: string[] | null;
  audio_prepare_ms?: number | null;
  base64_ms?: number | null;
  whisper_ms?: number | null;
  sanitizer_ms?: number | null;
  files_upload_ms?: number | null;
  files_poll_ms?: number | null;
  files_poll_count?: number | null;
  gemini_generate_ms?: number | null;
  gemini_delete_ms?: number | null;
  strict_literals_ms?: number | null;
  clipboard_ms?: number | null;
  total_pipeline_ms?: number | null;
  gemini_transport?: string | null;
  pipeline_runs?: PipelineRun[];
}

export interface PipelineProgressEvent {
  operation_id: number;
  kind:
    | "audio_preparing"
    | "recognizing"
    | "provider_failed"
    | "fallback_started"
    | "refining"
    | "formatting"
    | "delivering"
    | "complete";
  run_id?: string | null;
  provider?: string | null;
  fallback_provider?: string | null;
  message?: string | null;
}

export interface AudioStorageConfig {
  custom_directory?: string | null;
  effective_directory: string;
  default_directory: string;
}

export async function getAudioStorageConfig(): Promise<AudioStorageConfig> {
  return invoke<AudioStorageConfig>("get_audio_storage_config");
}

export async function setAudioStorageDirectory(
  path: string | null,
): Promise<AudioStorageConfig> {
  return invoke<AudioStorageConfig>("set_audio_storage_directory", { path });
}

/** Opens Explorer with the saved audio file selected. */
export async function revealHistoryAudio(id: string): Promise<void> {
  await invoke<void>("reveal_history_audio", { id });
}

/** Reads the saved source audio as normalized bytes for in-app playback. */
export async function readHistoryAudio(id: string): Promise<Uint8Array<ArrayBuffer>> {
  const response = await invoke<BinaryIpcResponse>("read_history_audio", { id });
  return normalizeAudioBytes(response);
}

/** Regenerates a persisted transcription from its saved audio. */
export async function retryTranscription(id: string): Promise<string> {
  return invoke<string>("retry_transcription", { id });
}

export async function retryTranscriptionWithFallback(id: string): Promise<string> {
  return invoke<string>("retry_transcription_with_fallback", { id });
}

export async function undoAiEdit(id: string, version: "raw" | "refined"): Promise<"replaced_selection" | "copied_to_clipboard"> {
  return invoke("undo_ai_edit", { id, version });
}

export type ContextSourceKind = "application" | "window_title" | "domain" | "selection" | "caret_context" | "clipboard";
export type ContextPrivacy = "metadata_only" | "ephemeral_local" | "cloud_allowed";
export interface ContextPreferences {
  sources: Array<{ source: ContextSourceKind; enabled: boolean; privacy: ContextPrivacy }>;
  persist_raw_context: boolean;
  allow_context_to_cloud: boolean;
  max_context_chars: number;
}
export interface OutputProfile {
  id: string; name: string; enabled: boolean;
  matcher: { processes: string[]; executables: string[]; window_titles: string[]; domains: string[] };
  formatting_level?: "literal" | "smart" | "aggressive" | null;
  content_type?: string | null;
  style_instruction?: string | null;
  allow_context_to_cloud?: boolean | null;
}
export interface OutputPolicyConfig {
  formatting_level: "literal" | "smart" | "aggressive";
  destination: "focused_field" | "clipboard_only" | "scratchpad";
  profiles: OutputProfile[];
  temporary_override?: string | null;
}
export interface VoiceSnippet { id: string; trigger: string; expansion: string; enabled: boolean; require_activation_phrase: boolean }
export interface CorrectionEvent { id: string; before: string; after: string; count: number; timestamp_ms: number; status: string; context: { application?: string | null; domain?: string | null; profile_id?: string | null } }
export interface ScratchpadNote { id: string; created_at_ms: number; text: string; pipeline_run_id?: string | null; profile_id?: string | null }

export const getContextPreferences = () => invoke<ContextPreferences>("get_context_preferences");
export const setContextPreferences = (preferences: ContextPreferences) => invoke<ContextPreferences>("set_context_preferences", { preferences });
export const getOutputPolicyConfig = () => invoke<OutputPolicyConfig>("get_output_policy_config");
export const setOutputPolicyConfig = (config: OutputPolicyConfig) => invoke<OutputPolicyConfig>("set_output_policy_config", { config });
export const getSnippets = () => invoke<VoiceSnippet[]>("get_snippets");
export const setSnippets = (snippets: VoiceSnippet[]) => invoke<VoiceSnippet[]>("set_snippets", { snippets });
export const getVocabularySuggestions = () => invoke<CorrectionEvent[]>("get_vocabulary_suggestions");
export const resolveVocabularySuggestion = (id: string, accepted: boolean) => invoke<void>("resolve_vocabulary_suggestion", { id, accepted });
export const getScratchpadNotes = () => invoke<ScratchpadNote[]>("get_scratchpad_notes");
export const deleteScratchpadNote = (id: string) => invoke<boolean>("delete_scratchpad_note", { id });

/** Structured vocabulary category (Phase 06). */
export type VocabularyCategory =
  | "ai_model"
  | "provider"
  | "application"
  | "person"
  | "file"
  | "command"
  | "function"
  | "identifier"
  | "study_term"
  | "other";

export interface VocabularyTerm {
  canonical: string;
  aliases: string[];
  category: VocabularyCategory;
  strict: boolean;
  enabled: boolean;
}

export async function getVocabulary(): Promise<VocabularyTerm[]> {
  return invoke<VocabularyTerm[]>("get_vocabulary");
}

export async function setVocabulary(
  terms: VocabularyTerm[],
): Promise<VocabularyTerm[]> {
  return invoke<VocabularyTerm[]>("set_vocabulary", { terms });
}

/** Returns the persisted developer-mode flag. */
export async function getDevMode(): Promise<boolean> {
  return invoke<boolean>("get_dev_mode");
}

/** Persists the developer-mode flag (gates the request-inspection UI). */
export async function setDevMode(value: boolean): Promise<void> {
  await invoke<void>("set_dev_mode", { value });
}

/** Returns whether the semantic validator (sanitizer) is active. */
export async function getSanitizerEnabled(): Promise<boolean> {
  return invoke<boolean>("get_sanitizer_enabled");
}

/** Toggles the semantic validator on/off and persists the choice. */
export async function setSanitizerEnabled(value: boolean): Promise<void> {
  await invoke<void>("set_sanitizer_enabled", { value });
}

export type WidgetVisibilityMode = "auto" | "always";
export type WidgetDock = "bottom" | "left" | "right";
export interface WidgetPreferences {
  visibility_mode: WidgetVisibilityMode;
  dock: WidgetDock;
  display?: string | null;
}

export type GadgetVisualState =
  | "hidden"
  | "idle"
  | "hover"
  | "appearing"
  | "initializing"
  | "recording"
  | "stopping"
  | "processing"
  | "processing_long"
  | "success"
  | "no_speech"
  | "error";

export interface GadgetPresentation {
  visual_state: GadgetVisualState;
  generation: number;
}

export async function getWidgetPreferences(): Promise<WidgetPreferences> {
  return invoke<WidgetPreferences>("get_widget_preferences");
}

export async function setWidgetVisibilityMode(
  mode: WidgetVisibilityMode,
): Promise<WidgetPreferences> {
  return invoke<WidgetPreferences>("set_widget_visibility_mode", { mode });
}

/** Applies state-derived native visibility, size and focus-following monitor placement. */
export async function setGadgetVisualState(
  visualState: GadgetVisualState,
): Promise<GadgetPresentation> {
  return invoke<GadgetPresentation>("set_gadget_visual_state", { visualState });
}

/** Visible-pill rectangle of the gadget overlay, in logical pixels relative to
 *  the gadget window's top-left corner. */
export interface GadgetHitRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Confirms that React completed layout for the exact native presentation.
 * The backend rejects stale generations and forces the WebView/native surface
 * to repaint after the DOM dimensions are known.
 */
export async function acknowledgeGadgetRendered(
  presentation: GadgetPresentation,
  rect: GadgetHitRect,
): Promise<boolean> {
  return invoke<boolean>("acknowledge_gadget_rendered", { presentation, rect });
}

/* ------------------------------- Events ------------------------------- */

export type RecordingEventType =
  | "recording-started"
  | "recording-stopped"
  | "recording-cancelled"
  | "recording-idle";

export function onRecordingEvent(
  handler: (type: RecordingEventType, status: RecordingStatus) => void,
): Promise<UnlistenFn> {
  const subscribe = async (name: RecordingEventType) => {
    return listen<RecordingStatus>(name, (event) => handler(name, event.payload));
  };

  // listen() resolves once per subscription; we wrap all three and return
  // a single unlisten that tears them all down.
  return (async () => {
    const unlisteners = await Promise.all([
      subscribe("recording-started"),
      subscribe("recording-stopped"),
      subscribe("recording-cancelled"),
      subscribe("recording-idle"),
    ]);
    return () => unlisteners.forEach((u) => u());
  })();
}

/** Lista todos os microfones conectados no host. */
export async function listAudioDevices(): Promise<string[]> {
  return invoke<string[]>("list_audio_devices");
}

/** Retorna o nome do microfone selecionado (ou null se for o padrão). */
export async function getInputDevice(): Promise<string | null> {
  return invoke<string | null>("get_input_device");
}

/** Salva o microfone desejado (ou null para usar o padrão). */
export async function setInputDevice(device: string | null): Promise<void> {
  await invoke<void>("set_input_device", { device });
}

/** Inicia a captura de teste do microfone. */
export async function startMicTest(): Promise<void> {
  await invoke<void>("start_mic_test");
}

/** Para a captura de teste do microfone. */
export async function stopMicTest(): Promise<void> {
  await invoke<void>("stop_mic_test");
}

/** Escuta o nível de áudio emitido pelo teste de microfone (0.0 a 1.0). */
export function onMicTestLevel(handler: (level: number) => void): Promise<UnlistenFn> {
  return listen<number>("mic-test-level", (event) => handler(event.payload));
}

/* ---------------------------- Voice Insights ---------------------------- */

export type InsightPeriod = "today" | "last7_days" | "last30_days" | "all_time";
export interface RankedCount { label: string; count: number; percentage: number }
export interface ApplicationInsight { name: string; count: number; percentage: number; domains: RankedCount[] }
export interface FillerInsight { phrase: string; count: number; per_1000_words: number }
export interface CorrectionInsight { before: string; after: string; count: number; in_vocabulary: boolean }
export interface BackfillStatus {
  running: boolean;
  paused: boolean;
  processed: number;
  total: number;
  analyzed: number;
  unavailable_audio: number;
  last_error?: string | null;
}
export interface VoiceEvidenceItem {
  key: string;
  title: string;
  description: string;
  count: number;
  share: number;
  confidence: number;
}
export interface VoiceSignature {
  catchphrase?: string | null;
  content_word?: string | null;
  phrase?: string | null;
  connector?: string | null;
  opener?: string | null;
}
export interface VoiceProfileEvidence {
  statistics: {
    sessions: number;
    words: number;
    average_words_per_session?: number | null;
    average_duration_seconds?: number | null;
    average_wpm?: number | null;
    typical_wpm?: [number, number] | null;
    manual_corrections: number;
    self_corrections_per_1000_words?: number | null;
    vocabulary_variety_mattr?: number | null;
  };
  recurring_topics: VoiceEvidenceItem[];
  recurring_intents: VoiceEvidenceItem[];
  linguistic_patterns: VoiceEvidenceItem[];
  signature_candidates: VoiceSignature & { corrected_expression?: string | null };
  correction_patterns: VoiceEvidenceItem[];
  application_patterns: VoiceEvidenceItem[];
  workflow_patterns: VoiceEvidenceItem[];
  acoustic_patterns: VoiceEvidenceItem[];
  temporal_patterns: VoiceEvidenceItem[];
  trends: VoiceEvidenceItem[];
  coverage: {
    level: "collecting" | "basic" | "archetype" | "rich" | "high_confidence";
    overall_confidence: number;
    session_coverage: number;
    audio_coverage: number;
    words: number;
    sessions: number;
    next_level_words: number;
  };
}
export interface VoiceProfileAttempt {
  provider: string;
  model: string;
  status: string;
  duration_ms: number;
  error?: string | null;
  failure_type?: string | null;
  request_ms?: number | null;
  ttfb_ms?: number | null;
  reported_total_tokens?: number | null;
  reported_input_tokens?: number | null;
  reported_output_tokens?: number | null;
  reported_cost_usd?: number | null;
  generation_id?: string | null;
  bytes_sent?: number | null;
  sanitized_response?: string | null;
}
export interface InterpretedVoicePattern {
  title: string;
  description: string;
  confidence: number;
  evidence_keys: string[];
}
export interface VoiceProfileTopic {
  title: string;
  description: string;
  share: number;
}
export interface VoiceObservation extends InterpretedVoicePattern {
  type: "language" | "usage" | "acoustic" | "temporal";
}
export interface VoiceProfile {
  title: string;
  description: string;
  archetype: {
    title: string;
    subtitle: string;
    description: string;
    confidence: number;
    evidence_keys: string[];
  };
  personal_portrait: {
    summary: string;
    confidence: number;
    evidence_keys: string[];
    distinctive_habits: InterpretedVoicePattern[];
    usage_rhythms: InterpretedVoicePattern[];
  };
  signature: VoiceSignature;
  communication_patterns: InterpretedVoicePattern[];
  recurring_topics: VoiceProfileTopic[];
  interesting_observations: VoiceObservation[];
  suggested_experiments: InterpretedVoicePattern[];
  generated_at_ms: number;
  generated_at_word_count: number;
  next_update_word_count: number;
  profile_version: number;
  provider: string;
  model: string;
  request_ms: number;
  ttfb_ms?: number;
  reported_total_tokens?: number;
  reported_input_tokens?: number;
  reported_output_tokens?: number;
  reported_cost_usd?: number;
  generation_id?: string;
  bytes_sent: number;
  attempts?: VoiceProfileAttempt[];
  evidence_bundle: VoiceProfileEvidence;
  sanitized_prompt: string;
  sanitized_response: string;
  schema_validation: string;
}
export interface VoiceProfileGenerationTrace {
  generated_at_ms: number;
  evidence_bundle: VoiceProfileEvidence;
  sanitized_prompt: string;
  attempts: VoiceProfileAttempt[];
  schema_validation: string;
}
export interface MetricTrend {
  metric: string;
  current: number;
  previous: number;
  change_percent?: number | null;
  change_absolute: number;
}
export interface InsightsResponse {
  analysis_version: number;
  period: InsightPeriod;
  usage: {
    sessions: number; words: number; audio_duration_ms: number;
    average_wpm?: number | null; median_wpm?: number | null; typical_wpm?: [number, number] | null;
    manual_corrections: number; vocabulary_corrections: number;
  };
  language: {
    language: string;
    most_used_word?: RankedCount | null;
    most_used_content_word?: RankedCount | null;
    most_used_phrase?: RankedCount | null;
    catchphrase?: RankedCount | null;
    fillers: FillerInsight[];
    self_corrections_per_1000_words?: number | null;
    vocabulary_variety?: number | null;
    vocabulary_variety_label?: string | null;
    most_corrected?: CorrectionInsight | null;
    catchphrase_ready: boolean;
    profile_ready: boolean;
  };
  audio: {
    analyzed_sessions: number; coverage_percentage: number;
    lufs_median?: number | null; lufs_typical?: [number, number] | null;
    rms_dbfs_median?: number | null; peak_dbfs_median?: number | null;
    clipping_ratio?: number | null; silence_ratio?: number | null; speech_ratio?: number | null;
    estimated_snr_db?: number | null; average_pause_ms?: number | null;
    median_f0_hz?: number | null; pitch_variation?: string | null; capture_quality?: string | null;
  };
  applications: RankedCount[];
  application_details: ApplicationInsight[];
  domains: RankedCount[];
  categories: RankedCount[];
  temporal: {
    peak_hour?: number | null; peak_weekday?: number | null;
    current_streak_days: number; longest_streak_days: number;
    activity: Array<{ day: string; sessions: number; words: number }>;
  };
  trends: MetricTrend[];
  voice_evidence: VoiceProfileEvidence;
  profile_enabled: boolean;
  profile?: VoiceProfile | null;
  profile_generation?: VoiceProfileGenerationTrace | null;
  profile_progress_words: number;
  profile_required_words: number;
  profile_generation_ready: boolean;
  backfill: BackfillStatus;
  generated_at_ms: number;
}

export const getInsights = (period: InsightPeriod) => invoke<InsightsResponse>("get_insights", { period });
export const setInsightsBackfillPaused = (paused: boolean) => invoke<BackfillStatus>("set_insights_backfill_paused", { paused });
export const setAiVoiceProfileEnabled = (enabled: boolean) => invoke<void>("set_ai_voice_profile_enabled", { enabled });
export const generateAiVoiceProfile = () => invoke<VoiceProfile>("generate_ai_voice_profile");
export const addInsightCorrectionToVocabulary = (before: string, after: string) => invoke<void>("add_insight_correction_to_vocabulary", { before, after });


export interface HistoryPage { items: HistoryEntry[]; total: number; next_offset: number | null; total_words: number }
export const getHistoryPage = (query = "", offset = 0, limit = 50, deleted = false) => invoke<HistoryPage>("get_history_page", { query, offset, limit, deleted });
export const getHistoryDetail = (id: string) => invoke<HistoryEntry>("get_history_detail", { id });
