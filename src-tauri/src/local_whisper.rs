//! Offline fallback: whisper.cpp running on this computer.
//!
//! The model is downloaded once on request from the official whisper.cpp model
//! repository and kept under the app data directory. It is only used when every
//! cloud provider of the selected mode failed (no network, quota, bad key).

use std::path::PathBuf;
use std::sync::OnceLock;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

static MODELS_DIR: OnceLock<PathBuf> = OnceLock::new();
static CONTEXT: OnceLock<Mutex<Option<(LocalModel, whisper_rs::WhisperContext)>>> = OnceLock::new();
static DOWNLOADING: Mutex<bool> = Mutex::new(false);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalModel {
    Base,
    #[default]
    Small,
}

impl LocalModel {
    fn file_name(self) -> &'static str {
        match self {
            Self::Base => "ggml-base.bin",
            Self::Small => "ggml-small.bin",
        }
    }

    /// Lower bound used to reject truncated or HTML error downloads.
    fn minimum_bytes(self) -> u64 {
        match self {
            Self::Base => 140_000_000,
            Self::Small => 460_000_000,
        }
    }

    fn url(self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.file_name()
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalModelStatus {
    pub model: LocalModel,
    pub installed: bool,
    pub size_bytes: Option<u64>,
    pub downloading: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub model: LocalModel,
    pub downloaded: u64,
    pub total: Option<u64>,
}

pub fn init(data_dir: PathBuf) {
    let _ = MODELS_DIR.set(data_dir.join("models"));
}

fn model_path(model: LocalModel) -> Option<PathBuf> {
    MODELS_DIR.get().map(|dir| dir.join(model.file_name()))
}

pub fn status(model: LocalModel) -> LocalModelStatus {
    let size = model_path(model)
        .and_then(|path| std::fs::metadata(path).ok())
        .map(|metadata| metadata.len());
    LocalModelStatus {
        model,
        installed: size.is_some_and(|size| size >= model.minimum_bytes()),
        size_bytes: size,
        downloading: *DOWNLOADING.lock(),
    }
}

pub fn is_installed(model: LocalModel) -> bool {
    status(model).installed
}

/// Streams the model to a `.part` file and renames it only when complete.
pub async fn download(
    model: LocalModel,
    mut on_progress: impl FnMut(DownloadProgress),
) -> Result<LocalModelStatus, String> {
    {
        let mut downloading = DOWNLOADING.lock();
        if *downloading {
            return Err("Um modelo já está sendo baixado.".into());
        }
        *downloading = true;
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            *DOWNLOADING.lock() = false;
        }
    }
    let _reset = Reset;

    let target = model_path(model).ok_or("Pasta de dados indisponível")?;
    let parent = target.parent().ok_or("Pasta de modelos inválida")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let partial = target.with_extension("bin.part");

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;
    let mut response = client
        .get(model.url())
        .send()
        .await
        .map_err(|error| format!("Falha ao baixar o modelo: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "O servidor do modelo respondeu {}",
            response.status().as_u16()
        ));
    }
    let total = response.content_length();
    let mut file = std::fs::File::create(&partial).map_err(|error| error.to_string())?;
    let mut downloaded = 0u64;
    let mut last_report = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Download interrompido: {error}"))?
    {
        std::io::Write::write_all(&mut file, &chunk).map_err(|error| error.to_string())?;
        downloaded += chunk.len() as u64;
        if downloaded - last_report >= 2_000_000 {
            last_report = downloaded;
            on_progress(DownloadProgress {
                model,
                downloaded,
                total,
            });
        }
    }
    drop(file);
    if downloaded < model.minimum_bytes() {
        let _ = std::fs::remove_file(&partial);
        return Err("O download do modelo veio incompleto. Tente novamente.".into());
    }
    std::fs::rename(&partial, &target).map_err(|error| error.to_string())?;
    on_progress(DownloadProgress {
        model,
        downloaded,
        total,
    });
    log::info!(
        "local-whisper: model {:?} installed ({downloaded} bytes)",
        model
    );
    Ok(status(model))
}

pub fn remove(model: LocalModel) -> Result<LocalModelStatus, String> {
    if let Some(slot) = CONTEXT.get() {
        let mut guard = slot.lock();
        if guard.as_ref().is_some_and(|(loaded, _)| *loaded == model) {
            *guard = None;
        }
    }
    if let Some(path) = model_path(model).filter(|path| path.exists()) {
        std::fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(status(model))
}

/// Decodes the 16-bit PCM WAV this app writes (any rate is accepted only if it
/// is 16 kHz mono, which is what whisper.cpp expects).
pub fn pcm16_mono_16k(wav: &[u8]) -> Result<Vec<f32>, String> {
    if wav.len() < 44 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("A transcrição offline aceita apenas áudio WAV.".into());
    }
    let mut offset = 12;
    let mut format_ok = false;
    while offset + 8 <= wav.len() {
        let id = &wav[offset..offset + 4];
        let size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let body = offset + 8;
        if id == b"fmt " && body + 16 <= wav.len() {
            let format = u16::from_le_bytes([wav[body], wav[body + 1]]);
            let channels = u16::from_le_bytes([wav[body + 2], wav[body + 3]]);
            let rate = u32::from_le_bytes(wav[body + 4..body + 8].try_into().unwrap());
            let bits = u16::from_le_bytes([wav[body + 14], wav[body + 15]]);
            format_ok = format == 1 && channels == 1 && rate == 16_000 && bits == 16;
        } else if id == b"data" {
            if !format_ok {
                return Err("A transcrição offline precisa de WAV PCM 16 kHz mono.".into());
            }
            let end = (body + size).min(wav.len());
            return Ok(wav[body..end]
                .chunks_exact(2)
                .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
                .collect());
        }
        offset = body + size + (size % 2);
    }
    Err("WAV sem dados de áudio.".into())
}

/// Runs whisper.cpp on a background thread. Returns the plain transcript.
pub async fn transcribe(model: LocalModel, wav: Vec<u8>) -> Result<String, String> {
    let path = model_path(model)
        .filter(|_| is_installed(model))
        .ok_or("Modelo offline não instalado.")?;
    tokio::task::spawn_blocking(move || {
        let samples = pcm16_mono_16k(&wav)?;
        let slot = CONTEXT.get_or_init(|| Mutex::new(None));
        let mut guard = slot.lock();
        if guard.as_ref().is_none_or(|(loaded, _)| *loaded != model) {
            let context = whisper_rs::WhisperContext::new_with_params(
                &path,
                whisper_rs::WhisperContextParameters::default(),
            )
            .map_err(|error| format!("Não foi possível carregar o modelo offline: {error}"))?;
            *guard = Some((model, context));
        }
        let (_, context) = guard.as_ref().expect("context loaded above");
        let mut state = context
            .create_state()
            .map_err(|error| format!("whisper.cpp: {error}"))?;
        let mut params =
            whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("pt"));
        params.set_n_threads(
            std::thread::available_parallelism()
                .map(|threads| threads.get().min(8) as i32)
                .unwrap_or(4),
        );
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_print_special(false);
        state
            .full(params, &samples)
            .map_err(|error| format!("whisper.cpp: {error}"))?;
        let mut text = String::new();
        for index in 0..state.full_n_segments() {
            if let Some(segment) = state.get_segment(index) {
                if let Ok(value) = segment.to_str_lossy() {
                    text.push_str(&value);
                }
            }
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err("O modelo offline não detectou fala.".into());
        }
        Ok(text)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_app_wav_and_rejects_other_formats() {
        let wav = crate::audio::create_wav_buffer(&[0, 16384, -32768]);
        let samples = pcm16_mono_16k(&wav).unwrap();
        assert_eq!(samples, vec![0.0, 0.5, -1.0]);
        assert!(
            pcm16_mono_16k(b"ID3 not a wav file at all, definitely more than 44 bytes long")
                .is_err()
        );
    }
}
