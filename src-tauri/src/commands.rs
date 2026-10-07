use crate::models::{
    ApiKeysPayload, DeepgramMode, EngineConfigPayload, GadgetVisualState, SharedState,
    TranscriptionEngine, WidgetPreferences, WidgetVisibilityMode,
};
use crate::pipeline_contract::TranscriptionMode;
use serde::{Deserialize, Serialize};
use tauri::State;

/// Error type returned by every IPC command. Serializes to a plain
/// string so the TypeScript frontend receives a readable message
/// inside `invoke`'s rejected promise.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("invalid configuration payload: {0}")]
    InvalidPayload(String),
    #[error("recording already in progress")]
    AlreadyRecording,
    #[error("no recording in progress")]
    NotRecording,
    #[error("internal state error: {0}")]
    Internal(String),
}

impl Serialize for CommandError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&crate::redaction::message(&self.to_string()))
    }
}

/// Snapshot returned to the frontend after a configuration update so
/// the UI can confirm the persisted selection.
#[derive(Debug, Clone, Serialize)]
pub struct EngineConfigSnapshot {
    pub engine: TranscriptionEngine,
    pub sanitizer: crate::models::SanitizerModel,
    pub dual_engine: bool,
    pub reasoning_enabled: bool,
    pub reasoning_effort: String,
    pub deepgram_mode: DeepgramMode,
}

/// `update_engine_config`
///
/// Receives the manual engine and sanitizer selections from the
/// frontend and persists them into the global `AppState` under their
/// respective `RwLock` guards, as well as on-disk in settings.json.
#[tauri::command]
pub async fn update_engine_config(
    state: State<'_, SharedState>,
    payload: EngineConfigPayload,
) -> Result<EngineConfigSnapshot, CommandError> {
    log::info!(
        "update_engine_config: engine={:?} sanitizer={:?} dual_engine={} reasoning={} effort={} deepgram_mode={}",
        payload.engine,
        payload.sanitizer,
        payload.dual_engine,
        payload.reasoning_enabled,
        payload.reasoning_effort,
        payload.deepgram_mode.as_str()
    );

    let shared = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let _config = crate::models::CONFIG_LOCK.lock();
        crate::settings::save_engine_config_batch(
            Some(payload.engine),
            Some(payload.sanitizer),
            payload.dual_engine,
            payload.reasoning_enabled,
            payload.reasoning_effort.clone(),
            payload.deepgram_mode,
        )
        .map_err(CommandError::Internal)?;
        {
            *shared.engine.write() = payload.engine;
        }
        {
            *shared.sanitizer.write() = payload.sanitizer;
        }
        {
            *shared.dual_engine.write() = payload.dual_engine;
        }
        {
            *shared.reasoning_enabled.write() = payload.reasoning_enabled;
        }
        {
            *shared.reasoning_effort.write() = payload.reasoning_effort.clone();
        }
        {
            *shared.deepgram_mode.write() = payload.deepgram_mode;
        }

        Ok(EngineConfigSnapshot {
            engine: payload.engine,
            sanitizer: payload.sanitizer,
            dual_engine: payload.dual_engine,
            reasoning_enabled: payload.reasoning_enabled,
            reasoning_effort: payload.reasoning_effort,
            deepgram_mode: payload.deepgram_mode,
        })
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// `get_engine_config`
///
/// Returns the currently active transcription engine and sanitizer model selection.
#[tauri::command]
pub fn get_engine_config(state: State<'_, SharedState>) -> EngineConfigSnapshot {
    EngineConfigSnapshot {
        engine: *state.engine.read(),
        sanitizer: *state.sanitizer.read(),
        dual_engine: *state.dual_engine.read(),
        reasoning_enabled: *state.reasoning_enabled.read(),
        reasoning_effort: state.reasoning_effort.read().clone(),
        deepgram_mode: *state.deepgram_mode.read(),
    }
}

/// Product-pipeline configuration. `modes_enabled` remains in the payload only
/// for compatibility with older frontends and is normalized to `true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeConfigPayload {
    pub modes_enabled: bool,
    pub mode: TranscriptionMode,
    #[serde(default = "default_true_fallback")]
    pub gemini_fallback_to_whisper: bool,
    #[serde(default = "default_true_file_tagging")]
    pub file_tagging_enabled: bool,
    #[serde(default)]
    pub gemini_pipelines: crate::pipeline_contract::GeminiPipelineConfig,
}

fn default_true_fallback() -> bool {
    true
}

fn default_true_file_tagging() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct ModeConfigSnapshot {
    pub modes_enabled: bool,
    pub mode: TranscriptionMode,
    pub gemini_fallback_to_whisper: bool,
    pub file_tagging_enabled: bool,
    pub gemini_pipelines: crate::pipeline_contract::GeminiPipelineConfig,
    /// Human labels for UI (copywriting).
    pub mode_label: String,
    pub mode_description: String,
}

fn mode_copy(
    mode: TranscriptionMode,
    ultra_fast_provider: crate::pipeline_contract::UltraFastProvider,
) -> (&'static str, &'static str) {
    match mode {
        TranscriptionMode::UltraFast => match ultra_fast_provider {
            crate::pipeline_contract::UltraFastProvider::Groq => {
                ("Ultrarrápido", "Whisper direto via Groq")
            }
            crate::pipeline_contract::UltraFastProvider::OpenRouter => {
                ("Ultrarrápido", "Whisper via OpenRouter STT (Groq)")
            }
        },
        TranscriptionMode::FastAccurate => ("Rápido e preciso", "Transcrição direta com Gemini"),
        TranscriptionMode::Precise => ("Preciso", "Whisper e Gemini em paralelo"),
        TranscriptionMode::UltraPrecise => {
            ("Ultrapreciso", "Whisper, validador e Gemini em sequência")
        }
    }
}

#[tauri::command]
pub fn get_mode_config(state: State<'_, SharedState>) -> ModeConfigSnapshot {
    let mode = *state.transcription_mode.read();
    let pipelines = state.gemini_pipelines.read().clone();
    let (label, desc) = mode_copy(mode, pipelines.ultra_fast_provider);
    ModeConfigSnapshot {
        modes_enabled: *state.modes_enabled.read(),
        mode,
        gemini_fallback_to_whisper: *state.gemini_fallback_to_whisper.read(),
        file_tagging_enabled: *state.file_tagging_enabled.read(),
        gemini_pipelines: pipelines,
        mode_label: label.to_string(),
        mode_description: desc.to_string(),
    }
}

#[tauri::command]
pub async fn update_mode_config(
    state: State<'_, SharedState>,
    payload: ModeConfigPayload,
) -> Result<ModeConfigSnapshot, CommandError> {
    for choice in [
        &payload.gemini_pipelines.fast_accurate,
        &payload.gemini_pipelines.precise,
        &payload.gemini_pipelines.ultra_precise,
    ] {
        choice
            .resolved_model_id()
            .map_err(CommandError::InvalidPayload)?;
    }

    log::info!(
        "update_mode_config: mode={:?} gemini_fallback={}",
        payload.mode,
        payload.gemini_fallback_to_whisper
    );

    let shared = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let _config = crate::models::CONFIG_LOCK.lock();
        crate::settings::save_mode_config_batch(
            true,
            payload.mode,
            payload.gemini_fallback_to_whisper,
            payload.file_tagging_enabled,
            payload.gemini_pipelines.clone(),
        )
        .map_err(CommandError::Internal)?;
        *shared.modes_enabled.write() = true;
        *shared.transcription_mode.write() = payload.mode;
        *shared.gemini_fallback_to_whisper.write() = payload.gemini_fallback_to_whisper;
        *shared.file_tagging_enabled.write() = payload.file_tagging_enabled;
        *shared.gemini_pipelines.write() = payload.gemini_pipelines.clone();

        let (label, desc) = mode_copy(payload.mode, payload.gemini_pipelines.ultra_fast_provider);
        Ok(ModeConfigSnapshot {
            modes_enabled: true,
            mode: payload.mode,
            gemini_fallback_to_whisper: payload.gemini_fallback_to_whisper,
            file_tagging_enabled: payload.file_tagging_enabled,
            gemini_pipelines: payload.gemini_pipelines,
            mode_label: label.to_string(),
            mode_description: desc.to_string(),
        })
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// Performs only provider-agnostic safety validation on a non-empty API key.
/// Provider key formats are opaque and may change; authentication is verified
/// by the provider when the credential is first used.
fn validate_api_key(provider: &str, key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Ok(());
    }
    if key.chars().any(char::is_control) {
        return Err(format!(
            "A chave da API do {} contém caracteres de controle inválidos.",
            provider
        ));
    }
    Ok(())
}

#[cfg(test)]
mod api_key_validation_tests {
    use super::validate_api_key;

    #[test]
    fn accepts_opaque_provider_key_formats() {
        assert!(validate_api_key("deepgram", "0123456789abcdef0123456789abcdef01234567").is_ok());
        assert!(validate_api_key("google", "future-format-without-known-prefix").is_ok());
        assert!(validate_api_key("groq", "another-opaque-format").is_ok());
    }

    #[test]
    fn rejects_embedded_control_characters() {
        assert!(validate_api_key("google", "valid-part\nsecond-part").is_err());
    }
}

/// `save_api_keys`
///
/// Replaces the stored API keys atomically: updates the in-memory `ApiKeys`
/// struct and persists the same snapshot to `api_keys.json` so the keys
/// survive an app restart or machine reboot. Keys are kept as plain `String`
/// — the masking is strictly a UI concern.
#[tauri::command]
pub async fn save_api_keys(
    state: State<'_, SharedState>,
    payload: ApiKeysPayload,
) -> Result<(), CommandError> {
    log::info!(
        "save_api_keys: groq={} google={} deepgram={} openrouter={} meta={}",
        payload.groq.len(),
        payload.google.len(),
        payload.deepgram.len(),
        payload.openrouter.len(),
        payload.meta.len(),
    );

    // Treat provider credentials as opaque. Do not reject valid keys based on
    // guessed prefixes or shapes (Deepgram keys, for example, are not UUIDs).
    for (provider, keys) in [
        ("groq", &payload.groq),
        ("google", &payload.google),
        ("deepgram", &payload.deepgram),
        ("openrouter", &payload.openrouter),
        ("meta", &payload.meta),
    ] {
        for key in keys {
            validate_api_key(provider, key.trim()).map_err(CommandError::InvalidPayload)?;
        }
    }

    let shared = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let _config = crate::models::CONFIG_LOCK.lock();
        let mut guard = shared.api_keys.write();
        let resolve = crate::secrets::resolve;
        let keys = crate::models::ApiKeys {
            groq: resolve("groq", payload.groq, &guard.groq).map_err(CommandError::Internal)?,
            google: resolve("google", payload.google, &guard.google)
                .map_err(CommandError::Internal)?,
            deepgram: resolve("deepgram", payload.deepgram, &guard.deepgram)
                .map_err(CommandError::Internal)?,
            openrouter: resolve("openrouter", payload.openrouter, &guard.openrouter)
                .map_err(CommandError::Internal)?,
            meta: resolve("meta", payload.meta, &guard.meta).map_err(CommandError::Internal)?,
        }
        .normalized();
        crate::secrets::save(&keys).map_err(CommandError::Internal)?;
        crate::redaction::register(&keys);
        *guard = keys;
        Ok(())
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// `get_api_keys`
///
/// Returns the currently stored API keys so the settings screen can populate
/// its input fields on load (the keys are read back from `api_keys.json` at
/// startup into the in-memory state).
#[tauri::command]
pub async fn get_api_keys(
    state: State<'_, SharedState>,
) -> Result<crate::models::ApiKeys, CommandError> {
    let shared = state.inner().clone();
    let keys = tokio::task::spawn_blocking(move || crate::secrets::mask(&shared.api_keys.read()))
        .await
        .map_err(|e| CommandError::Internal(e.to_string()))?;
    Ok(keys)
}

/// `transcribe_file`
///
/// Reads a local audio file selected/dropped in the frontend, transcribes it
/// through the engine currently active in settings, persists the source audio
/// to disk and appends a history entry (exactly like a microphone capture,
/// minus the clipboard injection). Returns the final sanitised text.
#[tauri::command]
pub async fn transcribe_file(
    state: State<'_, SharedState>,
    path: String,
) -> Result<String, CommandError> {
    log::info!("transcribe_file: {}", path);
    let shared = state.inner().clone();
    crate::audio::transcribe_file_path(&shared, path)
        .await
        .map_err(CommandError::Internal)
}

/// `evaluate_pronunciation`
///
/// Retrieves the saved audio for the history entry `id`, sends it together with
/// its transcribed text to Gemini Multimodal (using the stored Google API key)
/// and returns a Markdown speech assessment. The feedback is also persisted
/// onto the history entry so it survives a restart and re-opening the card.
#[tauri::command]
pub async fn evaluate_pronunciation(
    state: State<'_, SharedState>,
    id: String,
) -> Result<String, CommandError> {
    let lease = state
        .operations
        .begin("pronunciation")
        .map_err(CommandError::Internal)?;
    log::info!("evaluate_pronunciation: id={}", id);

    let entry = crate::history::get(&id)
        .ok_or_else(|| CommandError::Internal("histórico não encontrado".to_string()))?;

    let audio_path = entry.audio_path.ok_or_else(|| {
        CommandError::Internal("este item não possui áudio salvo para avaliar".to_string())
    })?;

    let google_key = state.next_google_key().ok_or_else(|| {
        CommandError::Internal("configure a chave de API do Google (Gemini) em Ajustes".to_string())
    })?;

    let audio_bytes = crate::audio_store::read(&audio_path).map_err(CommandError::Internal)?;

    let ext = std::path::Path::new(&audio_path)
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wav".to_string());

    let feedback = tokio::select! { biased;
        _ = lease.cancelled() => return Err(CommandError::Internal("Avaliação cancelada".into())),
        result = crate::gemini::evaluate_pronunciation(audio_bytes, &ext, &entry.text, &google_key) => result.map_err(CommandError::Internal)?,
    };

    // Persist the feedback so it is shown again without re-calling Gemini.
    if !crate::history::set_evaluation(&id, &feedback) {
        return Err(CommandError::Internal(
            "Não foi possível salvar a avaliação".into(),
        ));
    }

    Ok(feedback)
}

/// `toggle_recording_state`
///
/// Manual toggle used by the UI when the user clicks the record
/// button. Routes through the exact same pipeline as the global
/// `<Ctrl+B>` shortcut ([`crate::shortcuts::handle_toggle`]): starting
/// a capture opens the microphone and begins accumulating samples;
/// stopping it tears the stream down and dispatches the audio to the
/// active transcription engine, sanitises the text, copies it to the
/// clipboard and pastes it into the focused field. Returns the new
/// recording flag so the UI can update its timer immediately (the
/// `recording-*` Tauri events are emitted by `handle_toggle` too, for
/// cross-window sync).
#[tauri::command]
pub fn toggle_recording_state(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<bool, CommandError> {
    log::info!("toggle_recording_state (button) -> handle_toggle");
    let next = crate::shortcuts::handle_toggle(&app, state.inner());
    Ok(next)
}

/// Cancels the active capture through the same path as the global panic
/// shortcut. This lets the floating gadget discard a recording without
/// producing audio output or starting the transcription pipeline.
#[tauri::command]
pub fn cancel_recording(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), CommandError> {
    if state.operations.status().is_some_and(|job| {
        matches!(
            job.kind.as_str(),
            "import" | "export" | "archive" | "history-edit" | "voice-profile"
        )
    }) {
        return Err(CommandError::Internal(
            "Aguarde a gravação dos dados para preservar sua integridade".into(),
        ));
    }
    crate::shortcuts::handle_cancel(&app, state.inner());
    Ok(())
}

/// Versioned recording lifecycle snapshot used to reconcile event listeners
/// without allowing an older async response to overwrite a newer transition.
#[tauri::command]
pub fn get_recording_status(state: State<'_, SharedState>) -> crate::models::RecordingStatus {
    state.recording_status()
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioStorageConfig {
    pub custom_directory: Option<String>,
    pub effective_directory: String,
    pub default_directory: String,
}

fn audio_storage_snapshot() -> Result<AudioStorageConfig, CommandError> {
    let default = crate::audio_store::default_directory().ok_or_else(|| {
        CommandError::Internal("armazenamento de áudio ainda não inicializado".to_string())
    })?;
    let effective = crate::audio_store::effective_directory().unwrap_or_else(|| default.clone());
    Ok(AudioStorageConfig {
        custom_directory: crate::settings::load_audio_directory(),
        effective_directory: effective.to_string_lossy().into_owned(),
        default_directory: default.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub fn get_audio_storage_config() -> Result<AudioStorageConfig, CommandError> {
    audio_storage_snapshot()
}

/// Changes only the destination for future audio files. Existing history
/// entries retain their absolute paths and are neither moved nor deleted.
#[tauri::command]
pub async fn set_audio_storage_directory(
    path: Option<String>,
) -> Result<AudioStorageConfig, CommandError> {
    tokio::task::spawn_blocking(move || {
        let saved = match path.filter(|value| !value.trim().is_empty()) {
            Some(path) => Some(
                crate::audio_store::prepare_custom_directory(&path)
                    .map_err(CommandError::InvalidPayload)?
                    .to_string_lossy()
                    .into_owned(),
            ),
            None => None,
        };
        crate::settings::save_audio_directory(saved).map_err(CommandError::Internal)?;
        audio_storage_snapshot()
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// Opens Windows Explorer with the persisted source audio selected. Resolving
/// by history id keeps arbitrary paths out of the frontend IPC surface.
#[tauri::command]
pub async fn reveal_history_audio(id: String) -> Result<(), CommandError> {
    tokio::task::spawn_blocking(move || {
        let entry = crate::history::get(&id)
            .ok_or_else(|| CommandError::Internal("histórico não encontrado".to_string()))?;
        let path = entry.audio_path.ok_or_else(|| {
            CommandError::Internal("este item não possui áudio salvo".to_string())
        })?;
        let canonical = std::fs::canonicalize(&path).map_err(|e| {
            CommandError::Internal(format!("não foi possível localizar o áudio salvo: {e}"))
        })?;

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            std::process::Command::new("explorer.exe")
                .arg(format!("/select,{}", canonical.display()))
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| {
                    CommandError::Internal(format!("não foi possível abrir o Explorer: {e}"))
                })?;
            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = canonical;
            Err(CommandError::Internal(
                "mostrar o arquivo está disponível apenas no Windows".to_string(),
            ))
        }
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// Returns the persisted source audio as a raw IPC response for in-app
/// playback. Resolving the file through the history id avoids exposing an
/// arbitrary file-read command to the frontend.
#[tauri::command]
pub async fn read_history_audio(id: String) -> Result<tauri::ipc::Response, CommandError> {
    let bytes = tokio::task::spawn_blocking(move || {
        let entry = crate::history::get(&id)
            .ok_or_else(|| CommandError::Internal("histórico não encontrado".to_string()))?;
        let path = entry.audio_path.ok_or_else(|| {
            CommandError::Internal("este item não possui áudio salvo".to_string())
        })?;
        crate::audio_store::read(&path).map_err(CommandError::Internal)
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))??;

    Ok(tauri::ipc::Response::new(bytes))
}

/// `clear_history`
///
/// Wipes the persisted transcription history. Used by the "Limpar Tudo"
/// button in the Histórico view.
#[tauri::command]
pub async fn clear_history(state: State<'_, SharedState>) -> Result<(), CommandError> {
    let _lease = state
        .operations
        .begin("history-edit")
        .map_err(CommandError::Internal)?;
    tokio::task::spawn_blocking(crate::history::clear)
        .await
        .map_err(|e| CommandError::Internal(e.to_string()))?
        .map_err(CommandError::Internal)
}

/// Deletes a single history entry (and its audio file if present).
#[tauri::command]
pub async fn delete_history_entry(
    state: State<'_, SharedState>,
    id: String,
) -> Result<(), CommandError> {
    let _lease = state
        .operations
        .begin("history-edit")
        .map_err(CommandError::Internal)?;
    let ok = tokio::task::spawn_blocking(move || crate::history::delete_entry(&id))
        .await
        .map_err(|e| CommandError::Internal(e.to_string()))?;
    if ok {
        Ok(())
    } else {
        Err(CommandError::Internal(
            "não foi possível alterar o histórico; confira o armazenamento e tente novamente"
                .into(),
        ))
    }
}

/// Updates the final text of a history entry (manual edit). Keeps evaluation.
#[tauri::command]
pub async fn update_history_text(
    state: State<'_, SharedState>,
    id: String,
    text: String,
) -> Result<(), CommandError> {
    let _lease = state
        .operations
        .begin("history-edit")
        .map_err(CommandError::Internal)?;
    let entry = crate::history::get(&id)
        .ok_or_else(|| CommandError::Internal("entrada de histórico não encontrada".into()))?;
    let before = entry.text.clone();
    let learning_context = entry
        .pipeline_runs
        .last()
        .map(|run| crate::learning::CorrectionContext {
            application: run.context.process.clone(),
            domain: run.context.domain.clone(),
            profile_id: run.profile_id.clone(),
        })
        .unwrap_or_default();
    let prompts_enabled = state.features.read().learning_prompts;
    let app_handle = state.app_handle.read().clone();
    let ok = tokio::task::spawn_blocking(move || {
        let updated = crate::history::update_text(&id, &text);
        if updated {
            match crate::learning::record(&before, &text, learning_context) {
                Ok(Some(event)) if prompts_enabled && crate::learning::is_suggestion(&event) => {
                    // Offer the correction right away instead of waiting for the
                    // user to find it in Configurações.
                    if let Some(app) = app_handle.as_ref() {
                        let _ = tauri::Emitter::emit(app, "vocabulary-suggestion", &event);
                    }
                }
                Ok(_) => {}
                Err(error) => log::warn!("learning: failed to record correction: {}", error),
            }
        }
        updated
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?;
    if ok {
        Ok(())
    } else {
        Err(CommandError::Internal(
            "não foi possível alterar o histórico; confira o armazenamento e tente novamente"
                .into(),
        ))
    }
}

#[tauri::command]
pub async fn get_vocabulary_suggestions(
) -> Result<Vec<crate::learning::CorrectionEvent>, CommandError> {
    tokio::task::spawn_blocking(crate::learning::suggestions)
        .await
        .map_err(|error| CommandError::Internal(error.to_string()))
}

#[tauri::command]
pub async fn resolve_vocabulary_suggestion(
    state: State<'_, SharedState>,
    id: String,
    accepted: bool,
) -> Result<(), CommandError> {
    let event = tokio::task::spawn_blocking(move || crate::learning::resolve(&id, accepted))
        .await
        .map_err(|error| CommandError::Internal(error.to_string()))?
        .map_err(CommandError::Internal)?;
    if accepted {
        let event = event.ok_or_else(|| CommandError::Internal("suggestion not found".into()))?;
        let _config = crate::models::CONFIG_LOCK.lock();
        let mut vocabulary = state.vocabulary.read().clone();
        if let Some(term) = vocabulary
            .iter_mut()
            .find(|term| term.canonical == event.after)
        {
            if !term
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(&event.before))
            {
                term.aliases.push(event.before);
            }
        } else {
            vocabulary.push(crate::vocabulary::VocabularyTerm {
                canonical: event.after,
                aliases: vec![event.before],
                category: crate::vocabulary::VocabularyCategory::Other,
                strict: true,
                enabled: true,
            });
        }
        let vocabulary = crate::vocabulary::normalize_and_validate(vocabulary)
            .map_err(CommandError::InvalidPayload)?;
        crate::settings::save_vocabulary(vocabulary.clone()).map_err(CommandError::Internal)?;
        *state.vocabulary.write() = vocabulary;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_insights(
    state: State<'_, SharedState>,
    period: crate::insights::InsightPeriod,
) -> Result<crate::insights::InsightsResponse, CommandError> {
    let vocabulary = state.vocabulary.read().clone();
    let developer_mode = crate::settings::load_dev_mode();
    tokio::task::spawn_blocking(move || {
        let mut response = crate::insights::snapshot(period, &vocabulary);
        if !developer_mode {
            response.redact_developer_details();
        }
        response
    })
    .await
    .map_err(|error| CommandError::Internal(error.to_string()))
}

#[tauri::command]
pub fn set_insights_backfill_paused(paused: bool) -> crate::insights::BackfillStatus {
    crate::insights::set_backfill_paused(paused)
}

#[tauri::command]
pub async fn set_ai_voice_profile_enabled(enabled: bool) -> Result<(), CommandError> {
    tokio::task::spawn_blocking(move || crate::insights::set_profile_enabled(enabled))
        .await
        .map_err(|error| CommandError::Internal(error.to_string()))?
        .map_err(CommandError::Internal)
}

#[tauri::command]
pub async fn generate_ai_voice_profile(
    state: State<'_, SharedState>,
) -> Result<crate::insights::VoiceProfile, CommandError> {
    let _lease = state
        .operations
        .begin("voice-profile")
        .map_err(CommandError::Internal)?;
    crate::insights::generate_voice_profile(state.inner())
        .await
        .map_err(CommandError::Internal)
}

#[tauri::command]
pub async fn add_insight_correction_to_vocabulary(
    state: State<'_, SharedState>,
    before: String,
    after: String,
) -> Result<(), CommandError> {
    if before.trim().is_empty() || after.trim().is_empty() {
        return Err(CommandError::InvalidPayload(
            "Correção e grafia canônica são obrigatórias.".into(),
        ));
    }
    let _config = crate::models::CONFIG_LOCK.lock();
    let mut vocabulary = state.vocabulary.read().clone();
    if let Some(term) = vocabulary
        .iter_mut()
        .find(|term| term.canonical.eq_ignore_ascii_case(after.trim()))
    {
        if !term
            .aliases
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(before.trim()))
        {
            term.aliases.push(before.trim().to_string());
        }
    } else {
        vocabulary.push(crate::vocabulary::VocabularyTerm {
            canonical: after.trim().to_string(),
            aliases: vec![before.trim().to_string()],
            category: crate::vocabulary::VocabularyCategory::Other,
            strict: true,
            enabled: true,
        });
    }
    let vocabulary = crate::vocabulary::normalize_and_validate(vocabulary)
        .map_err(CommandError::InvalidPayload)?;
    crate::settings::save_vocabulary(vocabulary.clone()).map_err(CommandError::Internal)?;
    *state.vocabulary.write() = vocabulary;
    crate::learning::accept_pair(before.trim(), after.trim()).map_err(CommandError::Internal)?;
    Ok(())
}

/// `get_shortcuts`
///
/// Returns the currently active recording shortcuts so the Atalhos view can
/// display the live bindings on load.
#[tauri::command]
pub fn get_shortcuts(state: State<'_, SharedState>) -> crate::models::ShortcutConfig {
    state.shortcuts.read().clone()
}

/// `set_shortcuts`
///
/// Rebinds the global start/cancel recording shortcuts to the supplied key
/// combinations, re-registering them with the OS, persisting the choice and
/// returning the applied config. Rejects invalid/unavailable combinations
/// while keeping the previous binding intact.
#[tauri::command]
pub async fn set_shortcuts(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    toggle: String,
    cancel: String,
    command: Option<String>,
    hold_to_talk: Option<bool>,
) -> Result<crate::models::ShortcutConfig, CommandError> {
    log::info!("set_shortcuts: toggle={} cancel={}", toggle, cancel);
    let shared = state.inner().clone();
    let current = shared.shortcuts.read().clone();
    let requested = crate::models::ShortcutConfig {
        toggle,
        cancel,
        command: command.unwrap_or(current.command),
        hold_to_talk: hold_to_talk.unwrap_or(current.hold_to_talk),
    };
    tokio::task::spawn_blocking(move || {
        crate::shortcuts::apply_new(&app, &shared, requested).map_err(CommandError::Internal)
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// Full structured vocabulary.
#[tauri::command]
pub fn get_vocabulary(state: State<'_, SharedState>) -> Vec<crate::vocabulary::VocabularyTerm> {
    state.vocabulary.read().clone()
}

/// Replace structured vocabulary after validation.
#[tauri::command]
pub async fn set_vocabulary(
    state: State<'_, SharedState>,
    terms: Vec<crate::vocabulary::VocabularyTerm>,
) -> Result<Vec<crate::vocabulary::VocabularyTerm>, CommandError> {
    let cleaned =
        crate::vocabulary::normalize_and_validate(terms).map_err(CommandError::InvalidPayload)?;
    log::info!("set_vocabulary: {} terms", cleaned.len());

    let shared = state.inner().clone();
    let result = cleaned.clone();
    tokio::task::spawn_blocking(move || {
        let _config = crate::models::CONFIG_LOCK.lock();
        crate::settings::save_vocabulary(cleaned.clone()).map_err(CommandError::Internal)?;
        *shared.vocabulary.write() = cleaned;
        Ok::<(), CommandError>(())
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))??;

    Ok(result)
}

#[tauri::command]
pub fn get_widget_preferences(state: State<'_, SharedState>) -> WidgetPreferences {
    WidgetPreferences {
        visibility_mode: *state.widget_visibility_mode.read(),
        dock: *state.widget_dock.read(),
        display: crate::settings::load_widget_display(),
    }
}

#[tauri::command]
pub async fn set_widget_visibility_mode(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    mode: WidgetVisibilityMode,
) -> Result<WidgetPreferences, CommandError> {
    log::info!("set_widget_visibility_mode: {:?}", mode);
    let shared = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let _config = crate::models::CONFIG_LOCK.lock();
        crate::settings::save_widget_visibility_mode(mode)?;
        *shared.widget_visibility_mode.write() = mode;
        *shared.compact_mode.write() = mode == WidgetVisibilityMode::Always;
        Ok::<(), String>(())
    })
    .await
    .map_err(|error| CommandError::Internal(error.to_string()))?
    .map_err(CommandError::Internal)?;

    let current = *state.gadget_visual_state.read();
    if matches!(
        current,
        GadgetVisualState::Hidden | GadgetVisualState::Idle | GadgetVisualState::Hover
    ) {
        let target = if mode == WidgetVisibilityMode::Always {
            GadgetVisualState::Idle
        } else {
            GadgetVisualState::Hidden
        };
        crate::present_gadget(&app, state.inner(), target).map_err(CommandError::Internal)?;
    }

    let snapshot = get_widget_preferences(state);
    use tauri::Emitter;
    if let Err(error) = app.emit(
        crate::models::event_names::WIDGET_PREFERENCES_CHANGED,
        &snapshot,
    ) {
        log::warn!("failed to emit widget-preferences-changed: {}", error);
    }
    Ok(snapshot)
}

#[tauri::command]
pub fn set_gadget_visual_state(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    visual_state: GadgetVisualState,
) -> Result<crate::models::GadgetPresentation, CommandError> {
    crate::present_gadget(&app, state.inner(), visual_state).map_err(CommandError::Internal)
}

/// Confirms that the frontend laid out the exact native presentation returned
/// by `set_gadget_visual_state`. Acknowledgement also forces a native repaint,
/// closing the WebView2 gap where DOM frames exist but are not presented.
#[tauri::command]
pub fn acknowledge_gadget_rendered(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    presentation: crate::models::GadgetPresentation,
    rect: crate::models::GadgetHitRect,
) -> Result<bool, CommandError> {
    crate::acknowledge_gadget_rendered(&app, state.inner(), presentation, rect)
        .map_err(CommandError::Internal)
}

/// `get_recording_elapsed`
///
/// Returns the milliseconds elapsed since the current recording began, or `0`
/// if no recording is in progress. The frontend uses this to restore the
/// timer display after navigating away from Início and back.
#[tauri::command]
pub fn get_recording_elapsed(state: State<'_, SharedState>) -> u64 {
    state.recording_elapsed_ms()
}

/// `get_dev_mode`
///
/// Returns the persisted developer-mode flag so the settings screen and the
/// Histórico view can decide whether to expose the request-inspection panel.
#[tauri::command]
pub async fn get_dev_mode() -> Result<bool, CommandError> {
    tokio::task::spawn_blocking(crate::settings::load_dev_mode)
        .await
        .map_err(|e| CommandError::Internal(e.to_string()))
}

/// `set_dev_mode`
///
/// Persists the developer-mode flag. Raw request/response details are removed
/// from the history IPC payload while this flag is disabled.
#[tauri::command]
pub async fn set_dev_mode(value: bool) -> Result<(), CommandError> {
    log::info!("set_dev_mode: {}", value);
    tokio::task::spawn_blocking(move || {
        crate::settings::save_dev_mode(value).map_err(CommandError::Internal)?;
        Ok(())
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

#[tauri::command]
pub fn get_context_preferences(
    state: State<'_, SharedState>,
) -> crate::context::ContextPreferences {
    state.context_preferences.read().clone()
}

#[tauri::command]
pub async fn set_context_preferences(
    state: State<'_, SharedState>,
    mut preferences: crate::context::ContextPreferences,
) -> Result<crate::context::ContextPreferences, CommandError> {
    preferences.max_context_chars = preferences.max_context_chars.clamp(100, 4_000);
    let persisted = preferences.clone();
    let shared = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let _config = crate::models::CONFIG_LOCK.lock();
        crate::settings::save_context_preferences(persisted.clone())?;
        *shared.context_preferences.write() = persisted;
        Ok::<(), String>(())
    })
    .await
    .map_err(|error| CommandError::Internal(error.to_string()))?
    .map_err(CommandError::Internal)?;
    Ok(preferences)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputPolicyConfig {
    pub formatting_level: crate::output_policy::FormattingLevel,
    pub destination: crate::output_policy::DictationDestination,
    #[serde(default)]
    pub profiles: Vec<crate::output_policy::OutputProfile>,
    #[serde(default)]
    pub temporary_override: Option<String>,
}

#[tauri::command]
pub fn get_output_policy_config(state: State<'_, SharedState>) -> OutputPolicyConfig {
    OutputPolicyConfig {
        formatting_level: *state.formatting_level.read(),
        destination: *state.dictation_destination.read(),
        profiles: state.output_profiles.read().clone(),
        temporary_override: state.temporary_profile_override.read().clone(),
    }
}

#[tauri::command]
pub async fn set_output_policy_config(
    state: State<'_, SharedState>,
    config: OutputPolicyConfig,
) -> Result<OutputPolicyConfig, CommandError> {
    let mut ids = std::collections::HashSet::new();
    for profile in &config.profiles {
        let id = profile.id.trim();
        if id.is_empty() || !ids.insert(id.to_ascii_lowercase()) {
            return Err(CommandError::InvalidPayload(
                "profile ids must be non-empty and unique".into(),
            ));
        }
        if profile.name.trim().is_empty()
            || profile
                .style_instruction
                .as_deref()
                .is_some_and(|value| value.chars().count() > 2_000)
        {
            return Err(CommandError::InvalidPayload(
                "invalid profile name or style instruction".into(),
            ));
        }
    }
    if let Some(override_id) = config.temporary_override.as_deref() {
        if !config
            .profiles
            .iter()
            .any(|profile| profile.enabled && profile.id == override_id)
        {
            return Err(CommandError::InvalidPayload(
                "temporary override references an unavailable profile".into(),
            ));
        }
    }

    let persisted = config.clone();
    let shared = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let _config = crate::models::CONFIG_LOCK.lock();
        crate::settings::save_output_policy(&persisted)?;
        *shared.formatting_level.write() = persisted.formatting_level;
        *shared.dictation_destination.write() = persisted.destination;
        *shared.output_profiles.write() = persisted.profiles;
        *shared.temporary_profile_override.write() = persisted.temporary_override;
        Ok::<(), String>(())
    })
    .await
    .map_err(|error| CommandError::Internal(error.to_string()))?
    .map_err(CommandError::Internal)?;
    Ok(config)
}

#[tauri::command]
pub async fn get_scratchpad_notes() -> Result<Vec<crate::scratchpad::ScratchpadNote>, CommandError>
{
    tokio::task::spawn_blocking(crate::scratchpad::list)
        .await
        .map_err(|error| CommandError::Internal(error.to_string()))?
        .map_err(CommandError::Internal)
}

#[tauri::command]
pub async fn delete_scratchpad_note(id: String) -> Result<bool, CommandError> {
    tokio::task::spawn_blocking(move || crate::scratchpad::delete(&id))
        .await
        .map_err(|error| CommandError::Internal(error.to_string()))?
        .map_err(CommandError::Internal)
}

#[tauri::command]
pub async fn get_snippets() -> Result<Vec<crate::snippets::VoiceSnippet>, CommandError> {
    tokio::task::spawn_blocking(crate::snippets::list)
        .await
        .map_err(|error| CommandError::Internal(error.to_string()))?
        .map_err(CommandError::Internal)
}

#[tauri::command]
pub async fn set_snippets(
    snippets: Vec<crate::snippets::VoiceSnippet>,
) -> Result<Vec<crate::snippets::VoiceSnippet>, CommandError> {
    tokio::task::spawn_blocking(move || crate::snippets::replace(snippets))
        .await
        .map_err(|error| CommandError::Internal(error.to_string()))?
        .map_err(CommandError::InvalidPayload)
}

/// `get_sanitizer_enabled`
///
/// Returns whether the semantic validator (Stage 2 sanitization) is active.
/// When disabled, the raw acoustic transcription is delivered to the clipboard
/// without the Groq Chat Completions cleanup round-trip.
#[tauri::command]
pub fn get_sanitizer_enabled(state: State<'_, SharedState>) -> bool {
    *state.sanitizer_enabled.read()
}

/// `set_sanitizer_enabled`
///
/// Toggles the semantic validator on/off and persists the choice to
/// `settings.json`. Emitting the new value to the frontend is unnecessary
/// here: the only consumer is the settings toggle itself, which already
/// optimistically updated its local state.
#[tauri::command]
pub async fn set_sanitizer_enabled(
    state: State<'_, SharedState>,
    value: bool,
) -> Result<(), CommandError> {
    log::info!("set_sanitizer_enabled: {}", value);
    let shared = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let _config = crate::models::CONFIG_LOCK.lock();
        crate::settings::save_sanitizer_enabled(value).map_err(CommandError::Internal)?;
        *shared.sanitizer_enabled.write() = value;
        Ok(())
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// `list_audio_devices`
///
/// Queries CPAL for all available audio input devices on the host system.
#[tauri::command]
pub async fn list_audio_devices() -> Result<Vec<String>, CommandError> {
    tokio::task::spawn_blocking(|| {
        use cpal::traits::{DeviceTrait, HostTrait};
        let host = cpal::default_host();
        let devices = host.input_devices().map_err(|e| {
            CommandError::Internal(format!("Falha ao listar dispositivos de áudio: {}", e))
        })?;
        let mut names = Vec::new();
        for device in devices {
            if let Ok(name) = device.name() {
                names.push(name);
            }
        }
        Ok(names)
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// `get_input_device`
///
/// Returns the currently active input device selection from the settings store.
#[tauri::command]
pub async fn get_input_device() -> Option<String> {
    tokio::task::spawn_blocking(crate::settings::load_input_device)
        .await
        .unwrap_or_default()
}

/// `set_input_device`
///
/// Saves the user's manual input device selection to settings.
#[tauri::command]
pub async fn set_input_device(device: Option<String>) -> Result<(), CommandError> {
    log::info!("set_input_device: {:?}", device);
    tokio::task::spawn_blocking(move || {
        crate::settings::save_input_device(device).map_err(CommandError::Internal)?;
        Ok(())
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// `start_mic_test`
///
/// Instantiates a temporary input stream bound to the configured mic to pipe
/// loudness events back to the UI.
#[tauri::command]
pub async fn start_mic_test(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), CommandError> {
    log::info!("start_mic_test");
    let shared = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let lease = shared
            .operations
            .begin("mic-test")
            .map_err(CommandError::Internal)?;
        crate::audio::start_mic_test_stream(&app, &shared).map_err(CommandError::Internal)?;
        if shared.operations.status().is_some_and(|job| job.cancelled) {
            shared.test_stream.lock().take();
            return Err(CommandError::Internal(
                "Teste de microfone cancelado".into(),
            ));
        }
        *shared.test_lease.lock() = Some(lease);
        Ok(())
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// `stop_mic_test`
///
/// Tears down the active test stream and releases the microphone.
#[tauri::command]
pub async fn stop_mic_test(state: State<'_, SharedState>) -> Result<(), CommandError> {
    log::info!("stop_mic_test");
    let shared = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        *shared.test_stream.lock() = None;
        shared.test_lease.lock().take();
        Ok(())
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// `retry_transcription`
///
/// Retries a previously failed transcription using its persisted audio file.
#[tauri::command]
pub async fn retry_transcription(
    state: State<'_, SharedState>,
    id: String,
) -> Result<String, CommandError> {
    log::info!("retry_transcription: {}", id);
    let shared = state.inner().clone();
    crate::audio::retry_transcription_handler(&shared, &id)
        .await
        .map_err(CommandError::Internal)
}

#[tauri::command]
pub async fn retry_transcription_with_fallback(
    state: State<'_, SharedState>,
    id: String,
) -> Result<String, CommandError> {
    crate::audio::retry_transcription_handler_with_strategy(state.inner(), &id, true)
        .await
        .map_err(CommandError::Internal)
}

#[tauri::command]
pub async fn undo_ai_edit(
    state: State<'_, SharedState>,
    id: String,
    version: String,
) -> Result<crate::audio::UndoAiEditOutcome, CommandError> {
    crate::audio::undo_ai_edit(state.inner(), &id, &version)
        .await
        .map_err(CommandError::Internal)
}

/// Builds a `reg` Command pre-configured to never flash a console window.
///
/// On Windows, spawning any console application (.exe) from a GUI process
/// creates a visible `cmd.exe`-style window for the brief moment the child
/// is alive. When the user rapidly toggles between settings sub-tabs this
/// surfaces as an annoying CMD flicker, so we attach the
/// `CREATE_NO_WINDOW` creation flag to every `reg` invocation.
#[cfg(target_os = "windows")]
fn reg_command(args: &[&str]) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = std::process::Command::new("reg");
    cmd.args(args);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// `get_autostart`
///
/// Checks if the application is registered to start automatically on Windows.
///
/// Runs on `spawn_blocking` because spawning `reg.exe` and waiting for it
/// is a blocking operation that would otherwise stall the Tauri IPC
/// dispatcher thread and freeze the UI for a couple of seconds every time
/// the settings view mounts.
#[tauri::command]
pub async fn get_autostart() -> Result<bool, CommandError> {
    tokio::task::spawn_blocking(|| {
        #[cfg(target_os = "windows")]
        {
            let output = reg_command(&[
                "query",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                "/v",
                "Sonora",
            ])
            .output()
            .map_err(|e| CommandError::Internal(e.to_string()))?;
            Ok(output.status.success())
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok(false)
        }
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

/// `set_autostart`
///
/// Enables or disables the application autostart registry key on Windows.
///
/// Like `get_autostart`, runs off the IPC thread so the blocking `reg`
/// invocation never stalls the frontend.
#[tauri::command]
pub async fn set_autostart(enabled: bool) -> Result<(), CommandError> {
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "windows")]
        {
            if enabled {
                let exe_path = std::env::current_exe().map_err(|e| {
                    CommandError::Internal(format!(
                        "Não foi possível obter o caminho do executável: {}",
                        e
                    ))
                })?;
                let exe_path_str = exe_path.to_string_lossy();
                let val = format!("\"{}\" --autostart", exe_path_str);

                let status = reg_command(&[
                    "add",
                    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                    "/v",
                    "Sonora",
                    "/t",
                    "REG_SZ",
                    "/d",
                    &val,
                    "/f",
                ])
                .status()
                .map_err(|e| CommandError::Internal(e.to_string()))?;

                if !status.success() {
                    return Err(CommandError::Internal(
                        "Falha ao configurar inicialização no registro.".to_string(),
                    ));
                }
            } else {
                // Delete might fail if the key does not exist. We run it, but don't strictly crash if it fails because it's already removed.
                let _ = reg_command(&[
                    "delete",
                    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                    "/v",
                    "Sonora",
                    "/f",
                ])
                .status();
            }
            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok(())
        }
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn get_history_page(
    query: String,
    offset: usize,
    limit: usize,
    deleted: bool,
) -> Result<crate::history::Page, CommandError> {
    tokio::task::spawn_blocking(move || crate::history::page(&query, offset, limit, deleted))
        .await
        .map_err(|e| CommandError::Internal(e.to_string()))?
        .map_err(CommandError::Internal)
}
#[tauri::command]
pub fn get_history_detail(id: String) -> Result<crate::models::HistoryEntry, CommandError> {
    let mut entry = crate::history::get(&id)
        .ok_or_else(|| CommandError::Internal("Entrada não encontrada".into()))?;
    if !crate::settings::load_dev_mode() {
        entry.debug_info = None;
        for run in &mut entry.pipeline_runs {
            run.debug_info = None;
            for attempt in &mut run.attempts {
                attempt.result.request_sanitized = None;
                attempt.result.response_sanitized = None;
            }
        }
    }
    Ok(entry)
}
#[tauri::command]
pub fn restore_history_entry(id: String) -> Result<(), CommandError> {
    crate::history::restore_entry(&id).map_err(CommandError::Internal)
}
#[tauri::command]
pub fn repair_history_journal() -> Result<(), CommandError> {
    crate::history::repair_journal().map_err(CommandError::Internal)
}

#[tauri::command]
pub async fn get_local_diagnostics(
    state: State<'_, SharedState>,
) -> Result<crate::maintenance::Diagnostics, CommandError> {
    let shared = state.inner().clone();
    tokio::task::spawn_blocking(move || crate::maintenance::diagnostics(&shared))
        .await
        .map_err(|e| CommandError::Internal(e.to_string()))
}
#[tauri::command]
pub async fn retry_recovery_audio(
    state: State<'_, SharedState>,
    id: String,
) -> Result<String, CommandError> {
    crate::maintenance::retry_audio(state.inner(), id)
        .await
        .map_err(CommandError::Internal)
}
#[tauri::command]
pub async fn export_local_data(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    destination: String,
    include_audio: Option<bool>,
) -> Result<(), CommandError> {
    let lease = state
        .operations
        .begin("export")
        .map_err(CommandError::Internal)?;
    tokio::task::spawn_blocking(move || {
        let _lease = lease;
        let _config = crate::models::CONFIG_LOCK.lock();
        crate::maintenance::export(
            &app,
            std::path::Path::new(&destination),
            include_audio.unwrap_or(false),
        )
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
    .map_err(CommandError::Internal)
}
#[tauri::command]
pub async fn import_local_data(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    source: String,
) -> Result<usize, CommandError> {
    let _lease = state
        .operations
        .begin("import")
        .map_err(CommandError::Internal)?;
    let shared = state.inner().clone();
    let result = tokio::task::spawn_blocking(move || {
        let _config = crate::models::CONFIG_LOCK.lock();
        let result = crate::maintenance::import_history(&app, std::path::Path::new(&source))?;
        *shared.vocabulary.write() = crate::settings::load_vocabulary();
        *shared.output_profiles.write() = crate::settings::load_output_profiles();
        Ok::<_, String>(result)
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
    .map_err(CommandError::Internal)?;
    Ok(result)
}

#[tauri::command]
pub async fn archive_history_audio(
    state: State<'_, SharedState>,
    id: String,
    destination: String,
) -> Result<String, CommandError> {
    let _lease = state
        .operations
        .begin("archive")
        .map_err(CommandError::Internal)?;
    tokio::task::spawn_blocking(move || {
        crate::maintenance::archive_audio(&id, std::path::Path::new(&destination))
    })
    .await
    .map_err(|e| CommandError::Internal(e.to_string()))?
    .map_err(CommandError::Internal)
}

#[tauri::command]
pub fn get_feature_settings(state: State<'_, SharedState>) -> crate::assist::FeatureSettings {
    state.features.read().clone()
}

#[tauri::command]
pub async fn set_feature_settings(
    state: State<'_, SharedState>,
    features: crate::assist::FeatureSettings,
) -> Result<crate::assist::FeatureSettings, CommandError> {
    let shared = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let _config = crate::models::CONFIG_LOCK.lock();
        crate::settings::save_features(&features)?;
        *shared.features.write() = features.clone();
        Ok::<_, String>(features)
    })
    .await
    .map_err(|error| CommandError::Internal(error.to_string()))?
    .map_err(CommandError::Internal)
}

#[tauri::command]
pub fn get_local_model_status(
    model: crate::local_whisper::LocalModel,
) -> crate::local_whisper::LocalModelStatus {
    crate::local_whisper::status(model)
}

/// Downloads the offline Whisper model, emitting `local-model-progress`.
#[tauri::command]
pub async fn download_local_model(
    app: tauri::AppHandle,
    model: crate::local_whisper::LocalModel,
) -> Result<crate::local_whisper::LocalModelStatus, CommandError> {
    crate::local_whisper::download(model, |progress| {
        let _ = tauri::Emitter::emit(&app, "local-model-progress", &progress);
    })
    .await
    .map_err(CommandError::Internal)
}

#[tauri::command]
pub async fn remove_local_model(
    model: crate::local_whisper::LocalModel,
) -> Result<crate::local_whisper::LocalModelStatus, CommandError> {
    tokio::task::spawn_blocking(move || crate::local_whisper::remove(model))
        .await
        .map_err(|error| CommandError::Internal(error.to_string()))?
        .map_err(CommandError::Internal)
}

#[tauri::command]
pub async fn check_for_update(
    app: tauri::AppHandle,
) -> Result<crate::updates::UpdateInfo, CommandError> {
    crate::updates::check(&app)
        .await
        .map_err(CommandError::Internal)
}

/// Installs the downloaded update and restarts Sonora.
#[tauri::command]
pub async fn install_update(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), CommandError> {
    if state.recording_status().phase != crate::models::RecordingPhase::Idle {
        return Err(CommandError::Internal(
            "Termine a gravação antes de atualizar.".into(),
        ));
    }
    crate::updates::install(&app)
        .await
        .map_err(CommandError::Internal)
}
