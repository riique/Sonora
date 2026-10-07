//! Automatic updates from GitHub Releases (`latest.json` signed by the release
//! workflow). On startup a found update is installed right away, before the
//! user starts dictating; while the app runs, updates are downloaded quietly
//! and installed when the user chooses "Reiniciar" (or on the next start).

use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

const RECHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

static PENDING: Mutex<Option<(tauri_plugin_updater::Update, Vec<u8>)>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub available: Option<String>,
    pub notes: Option<String>,
    pub ready_to_install: bool,
}

fn info(app: &AppHandle, available: Option<&tauri_plugin_updater::Update>) -> UpdateInfo {
    UpdateInfo {
        current_version: app.package_info().version.to_string(),
        available: available.map(|update| update.version.clone()),
        notes: available.and_then(|update| update.body.clone()),
        ready_to_install: PENDING.lock().is_some(),
    }
}

/// Startup + periodic background checks, honoring the "auto_update" setting.
pub fn spawn(app: AppHandle, state: crate::models::SharedState) {
    tauri::async_runtime::spawn(async move {
        // Let the window and shortcuts settle first.
        tokio::time::sleep(Duration::from_secs(5)).await;
        let mut first = true;
        loop {
            if state.features.read().auto_update {
                let idle = state.recording_status().phase == crate::models::RecordingPhase::Idle
                    && state.operations.status().is_none();
                if let Err(error) = check_and_stage(&app, first && idle).await {
                    log::warn!("updates: {error}");
                }
            }
            first = false;
            tokio::time::sleep(RECHECK_EVERY).await;
        }
    });
}

async fn check_and_stage(app: &AppHandle, install_now: bool) -> Result<(), String> {
    let Some(update) = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    log::info!("updates: version {} available", update.version);
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;
    if install_now {
        log::info!("updates: installing {} at startup", update.version);
        update.install(bytes).map_err(|error| error.to_string())?;
        app.restart();
    }
    let ready = info(app, Some(&update));
    *PENDING.lock() = Some((update, bytes));
    let _ = app.emit("update-ready", &ready);
    Ok(())
}

/// Manual check from Configurações (works even with auto-update off).
pub async fn check(app: &AppHandle) -> Result<UpdateInfo, String> {
    if let Some((update, _)) = PENDING.lock().as_ref() {
        return Ok(info(app, Some(update)));
    }
    let update = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;
    Ok(info(app, update.as_ref()))
}

/// Installs the staged update (downloading it first when needed) and restarts.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let staged = PENDING.lock().take();
    let (update, bytes) = match staged {
        Some(staged) => staged,
        None => {
            let update = app
                .updater()
                .map_err(|error| error.to_string())?
                .check()
                .await
                .map_err(|error| error.to_string())?
                .ok_or("Nenhuma atualização disponível.")?;
            let bytes = update
                .download(|_, _| {}, || {})
                .await
                .map_err(|error| error.to_string())?;
            (update, bytes)
        }
    };
    update.install(bytes).map_err(|error| error.to_string())?;
    app.restart();
}
