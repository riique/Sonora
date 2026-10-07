//! Local diagnostics, recovery and portable backups. Credentials are excluded.
use crate::models::{AppState, HistoryEntry};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::Manager;

#[derive(Serialize)]
pub struct RecoveryAudio {
    pub id: String,
    pub bytes: u64,
}
#[derive(Serialize)]
pub struct Diagnostics {
    pub version: String,
    pub microphone: Option<String>,
    pub microphone_available: bool,
    pub missing_providers: Vec<String>,
    pub operation: Option<crate::operations::Status>,
    pub storage_errors: Vec<String>,
    pub recovery_audio: Vec<RecoveryAudio>,
}
fn recovery_dir() -> Result<PathBuf, String> {
    Ok(crate::audio_store::default_directory()
        .ok_or("Armazenamento indisponível")?
        .join("recovery"))
}
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 180
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}
pub fn diagnostics(state: &AppState) -> Diagnostics {
    use crate::pipeline_contract::{GeminiProvider, TranscriptionMode};
    use cpal::traits::{DeviceTrait, HostTrait};
    let microphone = crate::settings::load_input_device();
    let host = cpal::default_host();
    let microphone_available = match microphone.as_ref() {
        Some(name) => host.input_devices().is_ok_and(|mut devices| {
            devices.any(|device| device.name().is_ok_and(|device_name| device_name == *name))
        }),
        None => host.default_input_device().is_some(),
    };
    let keys = state.api_keys.read();
    let mode = *state.transcription_mode.read();
    let routes = state.gemini_pipelines.read();
    let mut required = vec![];
    if mode == TranscriptionMode::UltraFast {
        match routes.ultra_fast_provider {
            crate::pipeline_contract::UltraFastProvider::Groq => {
                required.push(("Groq", keys.groq.is_empty()));
            }
            crate::pipeline_contract::UltraFastProvider::OpenRouter => {
                required.push(("OpenRouter", keys.openrouter.is_empty()));
            }
        }
    } else {
        let route = match mode {
            TranscriptionMode::FastAccurate => &routes.fast_accurate,
            TranscriptionMode::Precise => &routes.precise,
            _ => &routes.ultra_precise,
        };
        required.push(match route.provider {
            GeminiProvider::GoogleAiStudio => ("Google", keys.google.is_empty()),
            GeminiProvider::OpenRouter => ("OpenRouter", keys.openrouter.is_empty()),
            GeminiProvider::Meta => ("Meta", keys.meta.is_empty()),
        });
        if mode != TranscriptionMode::FastAccurate {
            required.push(("Groq", keys.groq.is_empty()));
        }
    }
    let mut storage_errors = Vec::new();
    if let Err(error) = crate::secrets::load() {
        storage_errors.push(format!("Credenciais protegidas indisponíveis: {error}"));
    }
    if let Some(app) = state.app_handle.read().as_ref() {
        if let Ok(dir) = app.path().app_data_dir() {
            if dir.join("import-in-progress.json").exists() {
                storage_errors.push("Uma importação foi interrompida. Preserve os dados e restaure a pasta before-import indicada no marcador local antes de importar novamente.".into());
            }
            for name in STORES.iter().copied().chain(["voice-insights-v1.json"]) {
                let file = dir.join(name);
                if file.exists() {
                    if let Err(error) = crate::storage::read_json::<serde_json::Value>(&file) {
                        storage_errors.push(format!("{name}: {error}"));
                    }
                }
            }
        }
    }
    if let Err(error) = crate::history::page("", 0, 1, false) {
        storage_errors.push(error);
    }
    let mut recovery_audio = vec![];
    if let Ok(directory) = recovery_dir() {
        if let Ok(files) = fs::read_dir(directory) {
            for file in files.flatten() {
                let path = file.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("wav")
                    || path.with_extension("completed").exists()
                {
                    continue;
                }
                if let (Some(id), Ok(meta)) =
                    (path.file_stem().and_then(|s| s.to_str()), file.metadata())
                {
                    if safe_id(id) && meta.len() > 44 {
                        recovery_audio.push(RecoveryAudio {
                            id: id.into(),
                            bytes: meta.len(),
                        });
                    }
                }
            }
        }
    }
    recovery_audio.sort_by(|a, b| b.id.cmp(&a.id));
    Diagnostics {
        version: env!("CARGO_PKG_VERSION").into(),
        microphone,
        microphone_available,
        missing_providers: required
            .into_iter()
            .filter(|(_, missing)| *missing)
            .map(|(name, _)| name.into())
            .collect(),
        operation: state.operations.status(),
        storage_errors,
        recovery_audio,
    }
}
pub fn mark_capture_complete(session_id: &str) {
    if !safe_id(session_id) {
        return;
    }
    if let Ok(directory) = recovery_dir() {
        let path = directory.join(format!("recovery-{session_id}.completed"));
        if let Err(error) = crate::storage::atomic_write(&path, b"history_saved") {
            log::warn!("recovery marker: {error}");
        }
    }
}
pub async fn retry_audio(state: &Arc<AppState>, id: String) -> Result<String, String> {
    if !safe_id(&id) {
        return Err("Identificador de áudio inválido".into());
    }
    let lease = state.operations.begin("recovery")?;
    let source = recovery_dir()?.join(format!("{id}.wav"));
    let bytes = fs::read(&source).map_err(|e| e.to_string())?;
    if bytes.len() < 44 {
        return Err("Áudio incompleto".into());
    }
    let rate = u32::from_le_bytes(bytes[24..28].try_into().map_err(|_| "Áudio inválido")?);
    if !(8000..=192000).contains(&rate) {
        return Err("Taxa de áudio inválida".into());
    }
    let pcm = crate::capture_spool::read_pcm(&source)?;
    let pcm = crate::audio::resample(&pcm, rate, crate::audio::TARGET_SAMPLE_RATE);
    let path = source.with_extension("normalized.wav");
    crate::storage::atomic_write(&path, &crate::audio::create_wav_buffer(&pcm))?;
    let text = tokio::select! { biased;
        _ = lease.cancelled() => return Err("Recuperação cancelada; áudio preservado".into()),
        result = crate::audio::transcribe_file_path_inner(state, path.to_string_lossy().into_owned()) => result?,
    };
    crate::storage::atomic_write(&source.with_extension("completed"), b"history_saved")?;
    Ok(text)
}

#[derive(Serialize, Deserialize)]
pub struct Backup {
    format: String,
    version: u32,
    created_at_ms: u64,
    data: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    deleted_ids: Vec<String>,
    #[serde(default)]
    media: BTreeMap<String, Media>,
}
#[derive(Serialize, Deserialize)]
struct Media {
    file: String,
    bytes: u64,
}
const STORES: &[&str] = &[
    "settings.json",
    "snippets.json",
    "scratchpad.json",
    "vocabulary-learning.json",
    "shortcuts.json",
];
fn bundle(app: &tauri::AppHandle) -> Result<Backup, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let mut data = BTreeMap::new();
    for name in STORES {
        let file = dir.join(name);
        if file.exists() {
            data.insert(
                (*name).into(),
                crate::storage::read_json::<serde_json::Value>(&file)?,
            );
        }
    }
    let (entries, deleted_ids) = crate::history::export_entries()?;
    data.insert(
        "history.json".into(),
        serde_json::to_value(entries).map_err(|e| e.to_string())?,
    );
    Ok(Backup {
        format: "haumeavoice-portable-backup".into(),
        version: 2,
        created_at_ms: crate::pipeline_run::epoch_ms(),
        data,
        deleted_ids,
        media: BTreeMap::new(),
    })
}
pub fn export(
    app: &tauri::AppHandle,
    destination: &Path,
    include_audio: bool,
) -> Result<(), String> {
    export_bundle(bundle(app)?, destination, include_audio)
}
fn export_bundle(
    mut backup: Backup,
    destination: &Path,
    include_audio: bool,
) -> Result<(), String> {
    use std::io::Write;
    if !destination.is_absolute() || destination.exists() {
        return Err("Selecione um novo nome de arquivo absoluto".into());
    }
    let entries: Vec<HistoryEntry> = serde_json::from_value(
        backup
            .data
            .get("history.json")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .map_err(|_| "Histórico inválido")?;
    if include_audio {
        let media_dir = destination.with_extension("media");
        fs::create_dir(&media_dir).map_err(|e| format!("Use um nome de backup novo: {e}"))?;
        for (index, entry) in entries.iter().enumerate() {
            let Some(source) = entry.audio_path.as_ref().map(Path::new) else {
                continue;
            };
            let extension = source
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("wav");
            if !extension.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err("Extensão de áudio inválida".into());
            }
            let name = format!("audio-{index}.{extension}");
            let target = media_dir.join(&name);
            copy_new(source, &target)?;
            backup.media.insert(
                entry.id.clone(),
                Media {
                    file: name,
                    bytes: fs::metadata(&target).map_err(|e| e.to_string())?.len(),
                },
            );
        }
    }
    // Persist portable paths only: absolute paths from this computer have no authority on import.
    for entry in backup
        .data
        .get_mut("history.json")
        .and_then(|value| value.as_array_mut())
        .into_iter()
        .flatten()
    {
        entry["audio_path"] = serde_json::Value::Null;
    }
    let bytes = serde_json::to_vec(&backup).map_err(|e| e.to_string())?;
    if bytes.len() > 256 * 1024 * 1024 {
        return Err("Metadados do backup excedem 256 MiB".into());
    }
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
fn copy_new(source: &Path, target: &Path) -> Result<(), String> {
    let mut input = fs::File::open(source).map_err(|e| e.to_string())?;
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)
        .map_err(|e| e.to_string())?;
    std::io::copy(&mut input, &mut output)
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())
}
fn validated_media(source: &Path, media: &Media) -> Result<PathBuf, String> {
    if media.file.is_empty()
        || !media
            .file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'))
    {
        return Err("Nome de mídia inválido".into());
    }
    let directory = source
        .with_extension("media")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let path = directory
        .join(&media.file)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !path.starts_with(&directory) || path.parent() != Some(directory.as_path()) {
        return Err("Mídia fora da pasta do backup".into());
    }
    let meta = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() != media.bytes || meta.len() > 350 * 1024 * 1024 {
        return Err("Tamanho da mídia inválido".into());
    }
    Ok(path)
}
pub fn validate_backup(source: &Path) -> Result<Backup, String> {
    let meta = fs::metadata(source).map_err(|e| e.to_string())?;
    if meta.len() > 256 * 1024 * 1024 {
        return Err("Backup excede 256 MiB".into());
    }
    let backup: Backup = serde_json::from_slice(&fs::read(source).map_err(|e| e.to_string())?)
        .map_err(|_| "Backup inválido")?;
    if backup.format != "haumeavoice-portable-backup" || !matches!(backup.version, 1 | 2) {
        return Err("Formato de backup não suportado".into());
    }
    for (name, value) in &backup.data {
        match name.as_str() {
            "history.json" => {
                let _: Vec<HistoryEntry> = serde_json::from_value(value.clone())
                    .map_err(|_| "Histórico inválido no backup")?;
            }
            "settings.json" => crate::settings::validate_backup(value.clone())?,
            "snippets.json" => {
                let _: Vec<crate::snippets::VoiceSnippet> =
                    serde_json::from_value(value.clone()).map_err(|_| "Snippets inválidos")?;
            }
            "scratchpad.json" => {
                let _: Vec<crate::scratchpad::ScratchpadNote> =
                    serde_json::from_value(value.clone()).map_err(|_| "Notas inválidas")?;
            }
            "vocabulary-learning.json" => {
                let _: Vec<crate::learning::CorrectionEvent> =
                    serde_json::from_value(value.clone()).map_err(|_| "Vocabulário inválido")?;
            }
            "shortcuts.json" => {
                let _: crate::models::ShortcutConfig =
                    serde_json::from_value(value.clone()).map_err(|_| "Atalhos inválidos")?;
            }
            _ => return Err("Backup contém um recurso não permitido".into()),
        }
    }
    for media in backup.media.values() {
        validated_media(source, media)?;
    }
    Ok(backup)
}
pub fn import_history(app: &tauri::AppHandle, source: &Path) -> Result<usize, String> {
    let backup = validate_backup(source)?;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let marker = dir.join("import-in-progress.json");
    if marker.exists() {
        return Err("Restaure a importação interrompida antes de continuar".into());
    }
    let checkpoint = dir.join(format!("before-import-{}", crate::pipeline_run::epoch_ms()));
    fs::create_dir(&checkpoint).map_err(|e| e.to_string())?;
    for name in STORES
        .iter()
        .copied()
        .chain(["history.json", "history.events.jsonl"])
    {
        let original = dir.join(name);
        if original.exists() {
            copy_new(&original, &checkpoint.join(name))?;
        }
    }
    let rollback = dir.join(format!(
        "before-import-{}.json",
        crate::pipeline_run::epoch_ms()
    ));
    export(app, &rollback, false)?;
    // Prepare every merge before touching a store. Active provider, privacy,
    // destination and shortcut settings stay as chosen on this installation.
    let mut prepared = Vec::new();
    for name in [
        "settings.json",
        "snippets.json",
        "scratchpad.json",
        "vocabulary-learning.json",
    ] {
        let Some(incoming) = backup.data.get(name) else {
            continue;
        };
        let file = dir.join(name);
        let current: serde_json::Value = if file.exists() {
            crate::storage::read_json(&file)?
        } else if name == "settings.json" {
            serde_json::json!({})
        } else {
            serde_json::json!([])
        };
        let merged = if name == "settings.json" {
            crate::settings::merge_backup(current.clone(), incoming.clone())?
        } else {
            let mut entries = current
                .as_array()
                .cloned()
                .ok_or("Dados atuais inválidos")?;
            for entry in incoming.as_array().ok_or("Dados importados inválidos")? {
                if !entries.iter().any(|old| {
                    old["id"] == entry["id"]
                        || (name == "snippets.json" && old["trigger"] == entry["trigger"])
                }) {
                    entries.push(entry.clone());
                }
            }
            serde_json::Value::Array(entries)
        };
        prepared.push((file, current, merged));
    }
    let mut entries: Vec<HistoryEntry> = backup
        .data
        .get("history.json")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| "Histórico inválido")?
        .unwrap_or_default();
    let media_root = dir
        .join("audio")
        .join(format!("import-{}", crate::pipeline_run::epoch_ms()));
    if !backup.media.is_empty() {
        fs::create_dir_all(&media_root).map_err(|e| e.to_string())?;
    }
    for entry in &mut entries {
        entry.audio_path = None;
        if let Some(media) = backup.media.get(&entry.id) {
            let original = validated_media(source, media)?;
            let target = media_root.join(&media.file);
            copy_new(&original, &target)?;
            entry.audio_path = Some(target.to_string_lossy().into_owned());
        }
    }
    crate::storage::write_json(&marker, &serde_json::json!({"checkpoint": checkpoint}))?;
    let result = (|| {
        for (file, _, merged) in &prepared {
            crate::storage::write_json(file, merged)?;
        }
        crate::history::import_entries(entries, backup.deleted_ids)
    })();
    if result.is_err() {
        for (file, original, _) in &prepared {
            if let Err(error) = crate::storage::write_json(file, original) {
                return Err(format!("Importação interrompida; restaure o backup criado antes da importação. {error}"));
            }
        }
    }
    fs::remove_file(&marker).map_err(|e| e.to_string())?;
    result
}

/// Explicit retention action: verified copy, durable index update, then unlink
/// only the original audio inside the application's current storage roots.
pub fn archive_audio(id: &str, destination: &Path) -> Result<String, String> {
    if !destination.is_absolute() || !destination.is_dir() {
        return Err("Selecione uma pasta de arquivo existente".into());
    }
    let entry = crate::history::get_including_deleted(id)?;
    let source = PathBuf::from(entry.audio_path.ok_or("Entrada sem áudio")?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let roots: Vec<_> = [
        crate::audio_store::default_directory(),
        crate::audio_store::effective_directory(),
    ]
    .into_iter()
    .flatten()
    .filter_map(|path| path.canonicalize().ok())
    .collect();
    if !roots.iter().any(|root| source.starts_with(root)) {
        return Err(
            "Este áudio já está fora das pastas atuais. Use Mostrar áudio para gerenciá-lo.".into(),
        );
    }
    let extension = source
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or("Extensão inválida")?;
    if !["wav", "mp3", "m4a", "flac", "ogg", "aac", "webm", "mp4"]
        .contains(&extension.to_ascii_lowercase().as_str())
    {
        return Err("Formato de arquivo inválido".into());
    }
    let target = destination
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(format!(
            "sonora-archive-{}.{}",
            crate::pipeline_run::epoch_ms(),
            extension
        ));
    copy_new(&source, &target)?;
    if fs::read(&source).map_err(|e| e.to_string())?
        != fs::read(&target).map_err(|e| e.to_string())?
    {
        return Err("Verificação da cópia falhou; original preservado".into());
    }
    crate::history::archive_audio_path(id, target.to_string_lossy().into_owned())?;
    match fs::remove_file(&source) {
        Ok(()) => Ok("Áudio arquivado. O histórico continua associado à cópia verificada.".into()),
        Err(_) => Ok("Cópia verificada e associada ao histórico. O original também foi mantido porque estava em uso.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_backup_keeps_audio_association_and_rejects_bad_media() {
        let root = std::env::temp_dir().join(format!("sonora-backup-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let audio = root.join("source.wav");
        fs::write(&audio, crate::audio::create_wav_buffer(&[1, 2, -3])).unwrap();
        let mut data = BTreeMap::new();
        data.insert("history.json".into(), serde_json::json!([{"id":"synthetic", "date":"2026-09-04", "words":1, "engine":"test", "text":"synthetic", "audio_path":audio}]));
        let backup = Backup {
            format: "haumeavoice-portable-backup".into(),
            version: 2,
            created_at_ms: 0,
            data,
            deleted_ids: vec!["synthetic".into()],
            media: BTreeMap::new(),
        };
        let destination = root.join("test.json");
        export_bundle(backup, &destination, true).unwrap();
        let decoded = validate_backup(&destination).unwrap();
        assert_eq!(decoded.deleted_ids, vec!["synthetic"]);
        assert!(decoded.data["history.json"][0]["audio_path"].is_null());
        let media = decoded.media.get("synthetic").unwrap();
        let restored_audio = validated_media(&destination, media).unwrap();
        assert_eq!(fs::read(restored_audio).unwrap(), fs::read(&audio).unwrap());
        assert!(validated_media(
            &destination,
            &Media {
                file: "../source.wav".into(),
                bytes: 50
            }
        )
        .is_err());
        fs::write(
            destination.with_extension("media").join(&media.file),
            b"truncated",
        )
        .unwrap();
        assert!(validate_backup(&destination).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
