use crate::audio::{cancel_capture, start_capture, stop_capture};
use crate::models::{event_names, RecordingPhase, RecordingToggle, SharedState, ShortcutConfig};
use parking_lot::Mutex;
use std::{fs, path::PathBuf, sync::OnceLock};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Default shortcut strings, mirrored by [`ShortcutConfig::default`]. Parsed by
/// the `global-hotkey` crate via `TryFrom<&str>` for `Shortcut`.
pub const SHORTCUT_TOGGLE: &str = "Control+B";
pub const SHORTCUT_CANCEL: &str = "Control+Q";

static SHORTCUTS_PATH: OnceLock<PathBuf> = OnceLock::new();
static SHORTCUTS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Errors that may occur while (un)registering global shortcuts.
#[derive(Debug, thiserror::Error)]
pub enum ShortcutError {
    #[error("failed to register shortcut {shortcut}: {source}")]
    Register {
        shortcut: String,
        #[source]
        source: tauri_plugin_global_shortcut::Error,
    },
    #[error("failed to unregister all shortcuts: {0}")]
    UnregisterAll(tauri_plugin_global_shortcut::Error),
}

/* ----------------------------- Persistence ----------------------------- */

/// Called once during setup with the resolved `shortcuts.json` path.
pub fn init_store(file: PathBuf) {
    let _ = SHORTCUTS_PATH.set(file);
    let _ = SHORTCUTS_LOCK.set(Mutex::new(()));
}

/// Loads the persisted shortcut config, falling back to the defaults when the
/// file is missing or unparsable.
pub fn load_store() -> ShortcutConfig {
    let Some(lock) = SHORTCUTS_LOCK.get() else {
        return ShortcutConfig::default();
    };
    let _guard = lock.lock();
    let Some(file) = SHORTCUTS_PATH.get() else {
        return ShortcutConfig::default();
    };
    match fs::read_to_string(file) {
        Ok(contents) if !contents.trim().is_empty() => {
            serde_json::from_str(&contents).unwrap_or_default()
        }
        _ => ShortcutConfig::default(),
    }
}

/// Persists `cfg` to disk. Failures are logged, not propagated.
pub fn save_store(cfg: &ShortcutConfig) -> Result<(), String> {
    let _guard = SHORTCUTS_LOCK.get_or_init(|| Mutex::new(())).lock();
    let file = SHORTCUTS_PATH
        .get()
        .ok_or("Diretório de atalhos indisponível")?;
    crate::storage::write_json(file, cfg)
}

/// Core toggle logic shared by the IPC command path and the global
/// `<Ctrl+B>` shortcut handler.
///
/// Returns the requested recording flag immediately so callers on the main
/// thread (the global-shortcut callback) are never blocked. The
/// `recording-started` event is emitted only after CPAL confirms that the input
/// stream is running, so the UI is a reliable cue that speech is being captured.
///
/// **All blocking work** â€” COM microphone unmute, CPAL device acquisition,
/// stream playback â€” is dispatched to a background `std::thread::spawn` so
/// the Tauri event loop stays responsive. If the background work fails,
/// the recording state is reverted and a `recording-cancelled` event is
/// emitted.
pub fn handle_toggle(app: &AppHandle, state: &SharedState) -> bool {
    match state.toggle_recording_lifecycle() {
        RecordingToggle::Start(status) => {
            log::info!(
                "shortcut: start recording generation={} session={}",
                status.generation,
                status.session_id.as_deref().unwrap_or("-")
            );
            // Seed the session from the foreground application's monitor. The
            // native watcher keeps following focus for the entire visible flow.
            crate::begin_gadget_session(app, state);
            if let Err(e) = app.emit(event_names::RECORDING_INITIALIZING, &status) {
                log::warn!(
                    "failed to emit {}: {}",
                    event_names::RECORDING_INITIALIZING,
                    e
                );
            }
            // Heavy work (COM unmute + CPAL device open) runs off the main
            // thread to prevent AppHang when drivers stall.
            let app_bg = app.clone();
            let state_bg = state.clone();
            let generation = status.generation;
            let session_id = status.session_id.clone().unwrap_or_default();
            std::thread::spawn(move || {
                let start_requested_at = std::time::Instant::now();

                // Respect the endpoint mute chosen by the user.
                let capture_result = start_capture(&state_bg, generation, &session_id);
                let was_muted = false;

                match capture_result {
                    Ok(()) => {
                        let Some((accepted, ready_status)) =
                            state_bg.recording_capture_ready(generation)
                        else {
                            log::warn!(
                                "shortcut: capture ready for stale generation={}",
                                generation
                            );
                            return;
                        };
                        if !accepted {
                            log::info!(
                                "shortcut: capture became ready after stop/cancel generation={}; stop path owns cleanup",
                                generation
                            );
                            return;
                        }

                        state_bg.mark_recording_start();
                        if let Err(e) = app_bg.emit(event_names::RECORDING_STARTED, &ready_status) {
                            log::warn!("failed to emit {}: {}", event_names::RECORDING_STARTED, e);
                        }
                        crate::audio::spawn_audio_level_emitter(app_bg.clone(), state_bg.clone());
                        log::info!(
                            "shortcut: live audio-level emitter started for gadget waveform"
                        );
                        log::info!(
                            "shortcut: capture ready in {}ms (mic_was_muted={})",
                            start_requested_at.elapsed().as_millis(),
                            was_muted
                        );
                    }
                    Err(e) => {
                        let superseded = matches!(e, crate::audio::AudioError::Superseded);
                        if superseded {
                            log::info!(
                                "shortcut: capture start superseded generation={}",
                                generation
                            );
                        } else {
                            log::error!("shortcut: failed to start capture: {}", e);
                        }
                        state_bg.clear_recording_start();
                        let failed_status = state_bg.recording_capture_failed(generation);
                        cancel_capture(&state_bg);
                        if failed_status
                            .as_ref()
                            .is_some_and(|status| status.phase == RecordingPhase::Idle)
                        {
                            state_bg.capture_lease.lock().take();
                        }
                        if let Some(failed_status) = failed_status
                            .filter(|status| status.phase == RecordingPhase::Idle && !superseded)
                        {
                            if let Err(ee) =
                                app_bg.emit(event_names::RECORDING_CANCELLED, &failed_status)
                            {
                                log::warn!(
                                    "failed to emit {}: {}",
                                    event_names::RECORDING_CANCELLED,
                                    ee
                                );
                            }
                        }
                    }
                }
            });
            true
        }
        RecordingToggle::Stop(status) => {
            log::info!(
                "shortcut: stop recording generation={} session={}",
                status.generation,
                status.session_id.as_deref().unwrap_or("-")
            );
            state.clear_recording_start();
            // A global shortcut does not activate Sonora, so this captures the
            // exact app/window where the user requested stop. Delivery remains
            // tied to this HWND even when it lives on another monitor.
            let delivery_target = crate::context::capture_foreground_target();
            if let Err(e) = app.emit(event_names::RECORDING_STOPPED, &status) {
                log::warn!("failed to emit {}: {}", event_names::RECORDING_STOPPED, e);
            }
            let state_clone = state.clone();
            let app_clone = app.clone();
            let generation = status.generation;
            tauri::async_runtime::spawn(async move {
                while state_clone.recording_capture_start_pending(generation) {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                let lease = state_clone.capture_lease.lock().take();
                if let Some(lease) = lease {
                    tokio::select! { biased;
                        _ = lease.cancelled() => { cancel_capture(&state_clone); let _ = app_clone.emit("operation-cancelled", lease.id); }
                        _ = stop_capture(&state_clone, delivery_target) => {}
                    }
                }

                let finished = state_clone.finish_recording_stop(generation);
                if let Err(error) = app_clone.emit(event_names::RECORDING_IDLE, &finished) {
                    log::warn!("failed to emit {}: {}", event_names::RECORDING_IDLE, error);
                }
                log::info!(
                    "shortcut: recording lifecycle finished generation={} revision={} phase={:?}",
                    finished.generation,
                    finished.revision,
                    finished.phase
                );
            });
            false
        }
        RecordingToggle::Busy(status) => {
            log::warn!(
                "shortcut: toggle ignored while lifecycle is busy generation={} revision={} phase={:?}",
                status.generation,
                status.revision,
                status.phase
            );
            status.recording
        }
    }
}

/// Core cancel logic shared by the IPC command path and the global
/// `<Ctrl+Q>` panic shortcut handler. Forces the recording flag to
/// false, tears down the stream, discards the buffer without
/// producing any WAV output and emits the `recording-cancelled`
/// event so the UI resets the timer to 00:00.
pub fn handle_cancel(app: &AppHandle, state: &SharedState) {
    if state.operations.status().is_some_and(|job| {
        matches!(
            job.kind.as_str(),
            "import" | "export" | "archive" | "history-edit" | "voice-profile"
        )
    }) {
        return;
    }
    state.operations.cancel();
    if state
        .operations
        .status()
        .is_some_and(|job| job.kind == "mic-test")
    {
        state.test_stream.lock().take();
        state.test_lease.lock().take();
        return;
    }
    if state.recording_status().phase == RecordingPhase::Stopping {
        return;
    }

    let status = state.request_recording_cancel();
    if status.phase != RecordingPhase::Cancelling {
        log::info!(
            "shortcut: cancel ignored generation={} phase={:?}",
            status.generation,
            status.phase
        );
        return;
    }
    state.clear_recording_start();

    cancel_capture(state);

    log::info!(
        "shortcut: cancel recording generation={} session={}",
        status.generation,
        status.session_id.as_deref().unwrap_or("-")
    );

    if let Err(e) = app.emit(event_names::RECORDING_CANCELLED, &status) {
        log::warn!("failed to emit {}: {}", event_names::RECORDING_CANCELLED, e);
    }
    let idle = state.finish_recording_cancel(status.generation);
    if idle.phase == RecordingPhase::Idle {
        state.capture_lease.lock().take();
    }
    if idle.phase == RecordingPhase::Idle {
        if let Err(error) = app.emit(event_names::RECORDING_IDLE, &idle) {
            log::warn!("failed to emit {}: {}", event_names::RECORDING_IDLE, error);
        }
    }
}

fn recording_active(state: &SharedState) -> bool {
    matches!(
        state.recording_status().phase,
        RecordingPhase::Starting | RecordingPhase::Recording
    )
}

/// Applies a key transition of a recording shortcut. Toggle mode reacts to
/// presses only; hold-to-talk starts on press and stops on release. Key
/// auto-repeat sends extra presses while held, so hold mode ignores presses
/// while a recording is already running.
fn on_record_key(app: &AppHandle, state: &SharedState, pressed: bool, command: bool) {
    let hold = state.shortcuts.read().hold_to_talk;
    let active = recording_active(state);
    let should_toggle = match (hold, pressed) {
        (false, true) => true,
        (false, false) => false,
        (true, true) => !active,
        (true, false) => active,
    };
    if !should_toggle {
        return;
    }
    if command && !active {
        start_voice_command(app, state);
    } else {
        handle_toggle(app, state);
    }
}

/// Copies the current selection (restoring the user's clipboard afterwards),
/// then starts a recording whose transcript is treated as an instruction.
fn start_voice_command(app: &AppHandle, state: &SharedState) {
    let app = app.clone();
    let state = state.clone();
    std::thread::spawn(move || {
        let selection = capture_selection();
        log::info!(
            "shortcut: voice command started (selection: {} chars)",
            selection.as_ref().map_or(0, |text| text.chars().count())
        );
        *state.command_request.lock() = Some(crate::models::CommandRequest { selection });
        let main_app = app.clone();
        let main_state = state.clone();
        if app
            .run_on_main_thread(move || {
                if !handle_toggle(&main_app, &main_state) {
                    main_state.command_request.lock().take();
                }
            })
            .is_err()
        {
            state.command_request.lock().take();
        }
    });
}

fn capture_selection() -> Option<String> {
    use enigo::{
        Direction::{Click, Press, Release},
        Enigo, Key, Keyboard, Settings,
    };
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let previous = clipboard.get_text().ok();
    let _ = clipboard.clear();
    let mut enigo = Enigo::new(&Settings::default()).ok()?;
    // The command shortcut is usually still physically held (Ctrl+Shift+B):
    // release Shift/Alt first so the app receives Ctrl+C, not Ctrl+Shift+C.
    let _ = enigo.key(Key::Shift, Release);
    let _ = enigo.key(Key::Alt, Release);
    #[cfg(target_os = "windows")]
    let c_key = Key::C;
    #[cfg(not(target_os = "windows"))]
    let c_key = Key::Unicode('c');
    let _ = enigo.key(Key::Control, Press);
    let copied = enigo.key(c_key, Click);
    let _ = enigo.key(Key::Control, Release);
    copied.ok()?;
    std::thread::sleep(std::time::Duration::from_millis(180));
    let selection = clipboard
        .get_text()
        .ok()
        .filter(|text| !text.trim().is_empty());
    match previous {
        Some(previous) => {
            let _ = clipboard.set_text(previous);
        }
        None => {
            let _ = clipboard.clear();
        }
    }
    selection
}

/// Registers the global shortcuts. The handler closures clone the
/// `SharedState` handle so they can drive the audio pipeline without the
/// `State<'_>` injection (which is only valid inside a `#[tauri::command]`).
///
/// An invalid or already-claimed combination surfaces as
/// [`ShortcutError::Register`], which the caller uses to validate a rebind.
pub fn register_all(app: &AppHandle, config: &ShortcutConfig) -> Result<(), ShortcutError> {
    let toggle = config.toggle.as_str();
    let cancel = config.cancel.as_str();
    app.global_shortcut()
        .on_shortcut(toggle, move |app_h, _sc, event| {
            let s = app_h.state::<SharedState>().inner().clone();
            on_record_key(app_h, &s, event.state == ShortcutState::Pressed, false);
        })
        .map_err(|e| ShortcutError::Register {
            shortcut: toggle.to_string(),
            source: e,
        })?;

    app.global_shortcut()
        .on_shortcut(cancel, move |app_h, _sc, event| {
            if event.state == ShortcutState::Pressed {
                let s = app_h.state::<SharedState>().inner().clone();
                handle_cancel(app_h, &s);
            }
        })
        .map_err(|e| ShortcutError::Register {
            shortcut: cancel.to_string(),
            source: e,
        })?;

    let command = config.command.trim();
    if !command.is_empty() {
        app.global_shortcut()
            .on_shortcut(command, move |app_h, _sc, event| {
                let s = app_h.state::<SharedState>().inner().clone();
                on_record_key(app_h, &s, event.state == ShortcutState::Pressed, true);
            })
            .map_err(|e| ShortcutError::Register {
                shortcut: command.to_string(),
                source: e,
            })?;
    }

    log::info!(
        "global shortcuts registered: {} / {} / command {} (hold_to_talk={})",
        toggle,
        cancel,
        if command.is_empty() { "off" } else { command },
        config.hold_to_talk
    );
    Ok(())
}

/// Rebinds the recording shortcuts at runtime: clears all current
/// registrations, registers the requested combinations, and on success updates
/// the in-memory state and persists to disk. If the new combination is invalid
/// or unavailable, the previous binding is restored and an error is returned so
/// the user keeps a working shortcut.
pub fn apply_new(
    app: &AppHandle,
    state: &SharedState,
    requested: ShortcutConfig,
) -> Result<ShortcutConfig, String> {
    let cfg = ShortcutConfig {
        toggle: requested.toggle.trim().to_string(),
        cancel: requested.cancel.trim().to_string(),
        command: requested.command.trim().to_string(),
        hold_to_talk: requested.hold_to_talk,
    };

    if cfg.toggle.is_empty() || cfg.cancel.is_empty() {
        return Err("os atalhos não podem ficar vazios".to_string());
    }
    let distinct = [&cfg.toggle, &cfg.cancel, &cfg.command]
        .iter()
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let expected = if cfg.command.is_empty() { 2 } else { 3 };
    if distinct != expected {
        return Err("cada atalho precisa ser diferente dos outros".to_string());
    }

    // Start from a clean slate so the previous combinations are released.
    let _ = app.global_shortcut().unregister_all();

    match register_all(app, &cfg) {
        Ok(()) => {
            if let Err(error) = save_store(&cfg) {
                let previous = state.shortcuts.read().clone();
                let _ = app.global_shortcut().unregister_all();
                register_all(app, &previous)
                    .map_err(|restore| format!("{error}; falha ao restaurar atalhos: {restore}"))?;
                return Err(error);
            }
            *state.shortcuts.write() = cfg.clone();
            log::info!(
                "shortcuts: rebound to {} / {} / {}",
                cfg.toggle,
                cfg.cancel,
                cfg.command
            );
            Ok(cfg)
        }
        Err(e) => {
            // Restore the previous, known-good binding.
            let prev = state.shortcuts.read().clone();
            let _ = app.global_shortcut().unregister_all();
            let _ = register_all(app, &prev);
            Err(format!(
                "não foi possível registrar esse atalho (pode estar em uso por outro app): {}",
                e
            ))
        }
    }
}
