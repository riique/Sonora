//! Local mocks for Gemini audio services (no network).

use super::client::{extract_text, mime_for_ext, GenerateContentResponse};
use super::prompts::{transcription_prompt, TRANSCRIBE_PROMPT_VERSION};
use super::types::{GeminiGenerateResult, GeminiOperation};

/// Simulated remote file lifecycle for unit tests.
#[derive(Debug, Clone, Default)]
pub struct MockFilesApi {
    pub uploaded: bool,
    pub state: String,
    pub deleted: bool,
    pub fail_upload: bool,
    pub fail_processing: bool,
}

impl MockFilesApi {
    pub fn new_active() -> Self {
        Self {
            uploaded: false,
            state: "ACTIVE".into(),
            deleted: false,
            fail_upload: false,
            fail_processing: false,
        }
    }

    pub fn upload(&mut self, bytes: &[u8]) -> Result<String, String> {
        if self.fail_upload {
            return Err("mock upload failed".into());
        }
        if bytes.is_empty() {
            return Err("áudio vazio".into());
        }
        self.uploaded = true;
        if self.fail_processing {
            self.state = "FAILED".into();
            return Err("Gemini Files API falhou ao processar áudio: mock".into());
        }
        self.state = "ACTIVE".into();
        Ok("files/mock-1".into())
    }

    pub fn delete(&mut self, _name: &str) {
        self.deleted = true;
    }
}

/// End-to-end mock of transcribe without HTTP.
pub fn mock_transcribe(
    files: &mut MockFilesApi,
    audio: &[u8],
    ext: &str,
    model_text: &str,
) -> Result<GeminiGenerateResult, String> {
    let _mime = mime_for_ext(ext);
    let name = files.upload(audio)?;
    // simulate generate
    let text = model_text.trim().to_string();
    files.delete(&name);
    if text.is_empty() {
        return Err("o Gemini não retornou texto".into());
    }
    Ok(GeminiGenerateResult {
        operation: GeminiOperation::Transcribe,
        text,
        model: "gemini-3.5-flash-lite".into(),
        prompt_version: TRANSCRIBE_PROMPT_VERSION.into(),
        latency_ms: 1,
        remote_file_name: Some(name),
        transport: Some(super::transport::GeminiAudioTransport::FilesApi),
        timing: Default::default(),
        usage: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_transcribe_success_deletes_file() {
        let mut files = MockFilesApi::new_active();
        let r = mock_transcribe(&mut files, b"RIFF....", "wav", "olá mundo").unwrap();
        assert_eq!(r.text, "olá mundo");
        assert_eq!(r.operation, GeminiOperation::Transcribe);
        assert!(files.uploaded);
        assert!(files.deleted);
        assert!(r.prompt_version.contains("transcribe"));
    }

    #[test]
    fn mock_transcribe_empty_audio_fails_before_generate() {
        let mut files = MockFilesApi::new_active();
        let err = mock_transcribe(&mut files, b"", "wav", "x").unwrap_err();
        assert!(err.contains("vazio"));
        assert!(!files.deleted); // never uploaded
    }

    #[test]
    fn mock_upload_failure() {
        let mut files = MockFilesApi {
            fail_upload: true,
            ..MockFilesApi::new_active()
        };
        let err = mock_transcribe(&mut files, b"abc", "wav", "x").unwrap_err();
        assert!(err.contains("upload"));
    }

    #[test]
    fn mock_processing_failure_cleans_or_errors() {
        let mut files = MockFilesApi {
            fail_processing: true,
            ..MockFilesApi::new_active()
        };
        let err = mock_transcribe(&mut files, b"abc", "mp3", "x").unwrap_err();
        assert!(err.contains("processar") || err.contains("FAILED") || err.contains("falhou"));
    }

    #[test]
    fn response_json_extract() {
        let json = r#"{"candidates":[{"content":{"parts":[{"text":"ok"}]}}]}"#;
        let parsed: GenerateContentResponse = serde_json::from_str(json).unwrap();
        assert_eq!(extract_text(parsed).unwrap(), "ok");
    }

    #[test]
    fn transcription_prompt_loaded() {
        assert!(!transcription_prompt().system_instruction.is_empty());
    }
}
