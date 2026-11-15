//! Recovery of recordings that never reached the history (crash or forced exit).
use crate::models::AppState;
use serde::Serialize;
use std::{fs, path::PathBuf, sync::Arc};

#[derive(Serialize)]
pub struct RecoveryAudio {
    pub id: String,
    pub bytes: u64,
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
/// Lists interrupted recordings still waiting to be transcribed, newest first.
pub fn list_recovery_audio() -> Vec<RecoveryAudio> {
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
    recovery_audio
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
