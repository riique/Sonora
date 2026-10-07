//! Permanent on-disk storage for the audio that produced each transcription.
//!
//! Every transcription — whether captured from the microphone or uploaded as a
//! file — has its source audio copied into the configured directory (default:
//! `{app_data_dir}/audio`) as `{id}.{ext}`.
//! The resulting path is stored on the matching [`crate::models::HistoryEntry`]
//! so the pronunciation evaluator can later re-read the exact audio bytes and
//! hand them to Gemini.
//!
//! The default directory is resolved once during setup; the effective directory
//! is read from settings on each operation so changes apply without restarting.
//! Access is guarded by a process-wide `Mutex`.

use parking_lot::Mutex;
use std::{fs, path::PathBuf, sync::OnceLock};

static AUDIO_DIR: OnceLock<PathBuf> = OnceLock::new();
static AUDIO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Called once during app setup with the `audio` sub-directory of the app data
/// directory. Creates the directory eagerly so the first save never races on a
/// missing parent.
pub fn init(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);
    let _ = AUDIO_DIR.set(dir);
    let _ = AUDIO_LOCK.set(Mutex::new(()));
}

fn dir() -> Option<&'static PathBuf> {
    AUDIO_DIR.get()
}

pub fn default_directory() -> Option<PathBuf> {
    dir().cloned()
}

pub fn effective_directory() -> Option<PathBuf> {
    crate::settings::load_audio_directory()
        .map(PathBuf::from)
        .or_else(default_directory)
}

/// Validates and creates a user-selected audio directory without moving any
/// existing files. Returns the canonical absolute path persisted in settings.
pub fn prepare_custom_directory(path: &str) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(path.trim());
    if !candidate.is_absolute() {
        return Err("Selecione uma pasta com caminho absoluto.".to_string());
    }
    fs::create_dir_all(&candidate)
        .map_err(|e| format!("Não foi possível criar a pasta de áudio: {e}"))?;
    if !candidate.is_dir() {
        return Err("O local selecionado não é uma pasta.".to_string());
    }
    candidate
        .canonicalize()
        .map_err(|e| format!("Não foi possível validar a pasta de áudio: {e}"))
}

fn lock() -> &'static Mutex<()> {
    AUDIO_LOCK.get_or_init(|| Mutex::new(()))
}

/// Persists `bytes` as `{id}.{ext}` inside the audio directory and returns the
/// absolute path as a `String`. Returns `None` if the store was never
/// initialised or the write failed (the caller treats audio persistence as
/// best-effort: a failure must not abort the transcription itself).
pub fn save(id: &str, ext: &str, bytes: &[u8]) -> Option<String> {
    let _guard = lock().lock();
    let dir = effective_directory()?;
    if let Err(e) = fs::create_dir_all(&dir) {
        log::error!("audio_store: could not create audio dir: {}", e);
        return None;
    }
    let safe_ext = sanitize_ext(ext);
    let file = dir.join(format!("{}.{}", id, safe_ext));
    match crate::storage::atomic_write(&file, bytes) {
        Ok(()) => {
            log::info!("audio_store: saved {} bytes to {:?}", bytes.len(), file);
            Some(file.to_string_lossy().into_owned())
        }
        Err(e) => {
            log::error!("audio_store: failed to write audio file: {}", e);
            None
        }
    }
}

/// Preserves the unprocessed microphone WAV beside the canonical processed
/// audio as `{id}.original.{ext}`. The history entry continues to reference the
/// canonical file used for transcription; this sidecar is a rollback/debugging
/// source and is removed together with the history audio.
pub fn save_original(id: &str, ext: &str, bytes: &[u8]) -> Option<String> {
    let _guard = lock().lock();
    let dir = effective_directory()?;
    if let Err(e) = fs::create_dir_all(&dir) {
        log::error!("audio_store: could not create audio dir: {}", e);
        return None;
    }
    let safe_ext = sanitize_ext(ext);
    let file = dir.join(format!("{}.original.{}", id, safe_ext));
    match crate::storage::atomic_write(&file, bytes) {
        Ok(()) => {
            log::info!(
                "audio_store: preserved {} original bytes at {:?}",
                bytes.len(),
                file
            );
            Some(file.to_string_lossy().into_owned())
        }
        Err(e) => {
            log::error!("audio_store: failed to preserve original audio: {}", e);
            None
        }
    }
}

fn original_sidecar_path(canonical: &std::path::Path) -> Option<PathBuf> {
    let stem = canonical.file_stem()?.to_string_lossy();
    let extension = canonical.extension()?.to_string_lossy();
    Some(canonical.with_file_name(format!("{}.original.{}", stem, extension)))
}

/// Reads the audio bytes previously persisted at `path`. Returns an error
/// string (rather than panicking) so the evaluation command can surface a
/// readable diagnostic to the UI.
pub fn read(path: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("could not read saved audio at {}: {}", path, e))
}

/// Reads the microphone capture before normalization when that sidecar exists.
/// Voice Insights uses this path so capture-level metrics describe the input,
/// while transcription continues to use the canonical processed file.
pub fn read_original_or_canonical(path: &str) -> Result<Vec<u8>, String> {
    let canonical = PathBuf::from(path);
    if let Some(original) = original_sidecar_path(&canonical) {
        if original.is_file() {
            return fs::read(&original).map_err(|e| {
                format!(
                    "could not read original saved audio at {}: {}",
                    original.display(),
                    e
                )
            });
        }
    }
    read(path)
}

/// Maps a file extension to the MIME type sent to the transcription engines.
/// Defaults to a generic binary type for unknown extensions (the cloud APIs
/// still attempt container auto-detection in that case).
pub fn mime_for_ext(ext: &str) -> &'static str {
    match sanitize_ext(ext).as_str() {
        "wav" | "wave" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" | "mp4" | "aac" => "audio/mp4",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "webm" => "audio/webm",
        _ => "application/octet-stream",
    }
}

/// Strips anything that is not an ASCII alphanumeric from the extension and
/// lower-cases it, defaulting to `bin` when empty. Guards against a malicious
/// or malformed upload filename injecting path separators into the saved name.
fn sanitize_ext(ext: &str) -> String {
    let cleaned: String = ext
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    if cleaned.is_empty() {
        "bin".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_is_sanitized_without_path_components() {
        assert_eq!(sanitize_ext(".WAV"), "wav");
        assert_eq!(sanitize_ext("../../mp3"), "mp3");
        assert_eq!(sanitize_ext(""), "bin");
    }

    #[test]
    fn custom_directory_must_be_absolute() {
        assert!(prepare_custom_directory("relative/audio").is_err());
    }

    #[test]
    fn original_sidecar_is_derived_next_to_canonical_audio() {
        let canonical = PathBuf::from(r"C:\audio\123.wav");
        assert_eq!(
            original_sidecar_path(&canonical),
            Some(PathBuf::from(r"C:\audio\123.original.wav"))
        );
    }
}
