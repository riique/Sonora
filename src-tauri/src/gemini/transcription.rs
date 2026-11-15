//! Audio → text via Gemini multimodal (inline Base64 or Files API).

use base64::{engine::general_purpose, Engine as _};

use super::client::{
    adaptive_generate_timeout, build_file_request, build_inline_request,
    build_interaction_file_request, build_interaction_inline_request, generate_content_with_model,
    interact_with_model, is_transcribe_model, mime_for_ext,
};
use super::files::{spawn_cleanup, upload_and_wait};
use super::prompts::{fast_accurate_transcription_prompt, TRANSCRIBE_PROMPT_VERSION};
use super::transport::{
    estimate_wav_duration_ms, select_gemini_audio_transport, GeminiAudioTransport,
};
use super::types::{GeminiGenerateResult, GeminiOperation, GeminiStageTiming, TranscribeRequest};

/// Transcribes audio with Gemini. Short clips: inline Base64. Large: Files API.
/// Specialized models like `gemini-3.5-transcribe` are routed to the Interactions API
/// with native acoustic biasing (custom_vocabulary) and Smart mode.
pub async fn transcribe_audio(req: TranscribeRequest) -> Result<GeminiGenerateResult, String> {
    let t0 = std::time::Instant::now();
    let mime = mime_for_ext(&req.ext);
    let display = if req.display_name.trim().is_empty() {
        format!("sonora-stt.{}", req.ext)
    } else {
        req.display_name.clone()
    };

    let duration = req
        .duration_ms
        .or_else(|| estimate_wav_duration_ms(&req.audio_bytes));
    let transport = select_gemini_audio_transport(req.audio_bytes.len(), duration, mime)?;
    let generate_timeout = adaptive_generate_timeout(duration, req.audio_bytes.len());
    let is_transcribe = is_transcribe_model(&req.model);

    let (text, timing, remote, usage) = if is_transcribe {
        let custom_vocab =
            (!req.custom_vocabulary.is_empty()).then(|| req.custom_vocabulary.clone());
        let text_prompt: Option<&str> = None;

        match transport {
            GeminiAudioTransport::Inline => {
                let tb = std::time::Instant::now();
                let b64 = general_purpose::STANDARD.encode(&req.audio_bytes);
                let base64_ms = tb.elapsed().as_millis() as u64;
                let body = build_interaction_inline_request(
                    &req.model,
                    mime,
                    &b64,
                    text_prompt,
                    custom_vocab,
                );
                let generated = interact_with_model(&req.api_key, &body, generate_timeout).await?;
                (
                    generated.text,
                    GeminiStageTiming {
                        base64_ms: Some(base64_ms),
                        generate_ms: Some(generated.duration_ms),
                        ..Default::default()
                    },
                    None,
                    generated.usage,
                )
            }
            GeminiAudioTransport::FilesApi => {
                let (guard, up) =
                    upload_and_wait(&req.api_key, &req.audio_bytes, mime, &display).await?;
                let name = guard.name().to_string();
                let body = build_interaction_file_request(
                    &req.model,
                    guard.mime_type(),
                    guard.uri(),
                    text_prompt,
                    custom_vocab,
                );
                let gen = interact_with_model(&req.api_key, &body, generate_timeout).await;
                spawn_cleanup(guard);
                let generated = gen?;
                (
                    generated.text,
                    GeminiStageTiming {
                        files_upload_ms: Some(up.upload_ms),
                        files_poll_ms: Some(up.poll_ms),
                        files_poll_count: Some(up.poll_count),
                        generate_ms: Some(generated.duration_ms),
                        delete_ms: None, // async off critical path
                        ..Default::default()
                    },
                    Some(name),
                    generated.usage,
                )
            }
        }
    } else {
        let prompt = fast_accurate_transcription_prompt(&req.glossary_block);

        match transport {
            GeminiAudioTransport::Inline => {
                let tb = std::time::Instant::now();
                let b64 = general_purpose::STANDARD.encode(&req.audio_bytes);
                let base64_ms = tb.elapsed().as_millis() as u64;
                let body = build_inline_request(
                    &prompt.system_instruction,
                    &prompt.user_prompt,
                    mime,
                    &b64,
                )
                .for_fast_accurate();
                let generated =
                    generate_content_with_model(&req.api_key, &req.model, &body, generate_timeout)
                        .await?;
                (
                    generated.text,
                    GeminiStageTiming {
                        base64_ms: Some(base64_ms),
                        generate_ms: Some(generated.duration_ms),
                        ..Default::default()
                    },
                    None,
                    generated.usage,
                )
            }
            GeminiAudioTransport::FilesApi => {
                let (guard, up) =
                    upload_and_wait(&req.api_key, &req.audio_bytes, mime, &display).await?;
                let name = guard.name().to_string();
                let body = build_file_request(
                    &prompt.system_instruction,
                    &prompt.user_prompt,
                    guard.mime_type(),
                    guard.uri(),
                )
                .for_fast_accurate();
                let gen =
                    generate_content_with_model(&req.api_key, &req.model, &body, generate_timeout)
                        .await;
                spawn_cleanup(guard);
                let generated = gen?;
                (
                    generated.text,
                    GeminiStageTiming {
                        files_upload_ms: Some(up.upload_ms),
                        files_poll_ms: Some(up.poll_ms),
                        files_poll_count: Some(up.poll_count),
                        generate_ms: Some(generated.duration_ms),
                        delete_ms: None, // async off critical path
                        ..Default::default()
                    },
                    Some(name),
                    generated.usage,
                )
            }
        }
    };

    Ok(GeminiGenerateResult {
        operation: GeminiOperation::Transcribe,
        text,
        model: req.model,
        prompt_version: TRANSCRIBE_PROMPT_VERSION.to_string(),
        latency_ms: t0.elapsed().as_millis() as u64,
        remote_file_name: remote,
        transport: Some(transport),
        timing,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcribe_model_uses_interactions_request_builder() {
        let req = TranscribeRequest {
            audio_bytes: vec![0, 1, 2, 3],
            ext: "wav".into(),
            api_key: "test-key".into(),
            model: "gemini-3.5-transcribe".into(),
            display_name: "test.wav".into(),
            duration_ms: Some(1000),
            glossary_block: "termo".into(),
            custom_vocabulary: vec!["Sonora".into(), "Tauri".into()],
        };

        assert!(is_transcribe_model(&req.model));
        let interaction_body = build_interaction_inline_request(
            &req.model,
            "audio/wav",
            "AAEC",
            Some("contexto do editor"),
            Some(req.custom_vocabulary.clone()),
        );

        assert_eq!(interaction_body.model, "gemini-3.5-transcribe");
        assert_eq!(interaction_body.input.len(), 2);
        let config = interaction_body
            .generation_config
            .unwrap()
            .transcription_config
            .unwrap();
        assert_eq!(config.custom_vocabulary.unwrap(), vec!["Sonora", "Tauri"]);
        assert_eq!(config.mode.unwrap().mode_type, "smart");
        assert!(config.language_codes.is_none());
    }
}
