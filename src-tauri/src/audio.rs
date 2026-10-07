//! Audio capture, WAV encoding, clipboard delivery and history I/O.
//!
//! Transcription engine selection, dual STT, sanitization and metrics live in
//! [`crate::transcription`]. Capture still happens in RAM until the final WAV
//! is assembled and optionally persisted via [`crate::audio_store`].

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{Emitter, Manager};

use crate::models::AppState;

/// Emits the `transcribing` state boolean **only to the gadget window**,
/// which is the sole subscriber (see `GadgetView.tsx`). Broadcast emits via
/// `app.emit` also wake the much busier main window's IPC bridge for nothing
/// — extra pressure that compounds the gadget AppHang when the overlay is
/// backgrounded and the WebView2 JS task queue is throttled.
/// Best-effort: silently no-ops when the gadget window is gone.
fn emit_transcribing(state: &AppState, value: bool) {
    use tauri::{Emitter, Manager};
    let Some(job) = state
        .operations
        .status()
        .filter(|job| matches!(job.kind.as_str(), "microphone" | "retry-mic"))
    else {
        return;
    };
    if let Some(handle) = state.app_handle.read().as_ref() {
        if let Some(gadget) = handle.get_webview_window("gadget") {
            let _ = gadget.emit("transcribing", serde_json::json!({"active": value, "operation_id": job.id, "cancelled": job.cancelled}));
        }
    }
}

/// Target sample rate for the final WAV buffer. Cloud STT engines
/// such as Groq Whisper and Deepgram Nova-3 expect 16 kHz audio.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;
/// Number of channels in the output WAV. Always mono.
const CHANNELS: u16 = 1;
/// Bit depth of the output WAV. Always 16-bit signed PCM.
const BITS_PER_SAMPLE: u16 = 16;

pub fn spawn_audio_level_emitter(handle: tauri::AppHandle, state: Arc<AppState>) {
    let pending_emit = Arc::new(AtomicBool::new(false));

    std::thread::spawn(move || {
        while state.is_recording() {
            std::thread::sleep(std::time::Duration::from_millis(80));

            let fault = state.capture_fault.lock().take().or_else(|| {
                (state.recording_elapsed_ms() >= crate::capture_spool::MAX_CAPTURE_SECONDS * 1000)
                    .then(|| {
                        "Limite de 15 minutos atingido. Áudio preservado na recuperação.".into()
                    })
            });
            if let Some(error) = fault {
                crate::shortcuts::handle_cancel(&handle, &state);
                use tauri::Emitter;
                let _ = handle.emit("capture-error", error);
                break;
            }
            let capture_rate = *state.capture_rate.read();
            // Shorter analysis window → snappier reaction to speech onsets.
            let window = ((capture_rate as usize) / 20).max(256);
            let mut level = state.recent_level(window);

            // Gate only true silence / very low room tone, then expand speech
            // into the full 0..1 visual range. Lower threshold + steeper map
            // so quiet-to-normal talking visibly drives the gadget bars.
            level = if level < 0.012 {
                0.0
            } else {
                // Subtract gate, stretch by ~0.38 full-scale span, then gamma
                // < 1 so mid speech reads stronger without instantly clipping.
                ((level - 0.012) / 0.38).clamp(0.0, 1.0).powf(0.55)
            };

            if pending_emit.swap(true, Ordering::AcqRel) {
                continue;
            }

            let app_for_emit = handle.clone();
            let pending_for_emit = pending_emit.clone();
            let pending_for_error = pending_emit.clone();
            let queued = handle.run_on_main_thread(move || {
                use tauri::{Emitter, Manager};
                if let Some(gadget) = app_for_emit.get_webview_window("gadget") {
                    let _ = gadget.emit(crate::models::event_names::AUDIO_LEVEL, level);
                }
                pending_for_emit.store(false, Ordering::Release);
            });

            if queued.is_err() {
                pending_for_error.store(false, Ordering::Release);
            }
        }

        let app_for_emit = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            use tauri::{Emitter, Manager};
            if let Some(gadget) = app_for_emit.get_webview_window("gadget") {
                let _ = gadget.emit(crate::models::event_names::AUDIO_LEVEL, 0.0_f32);
            }
        });
    });
}

/// Errors that can occur while acquiring the microphone or building
/// the input stream. All variants are graceful: the caller should
/// emit a `recording-cancelled` event and keep the Tauri process
/// alive regardless of the failure mode.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no default input device is available on this host")]
    NoInputDevice,
    #[error("default input device failed to open: {0}")]
    DeviceOpen(String),
    #[error("unsupported sample format returned by the device: {0:?}")]
    UnsupportedSampleFormat(SampleFormat),
    #[error("failed to build the input stream: {0}")]
    StreamBuild(String),
    #[error("failed to start the input stream: {0}")]
    StreamPlay(String),
    #[error("recording start was superseded by a stop or cancel request")]
    Superseded,
}

/// Builds a 44-byte canonical RIFF/WAVE header followed by the
/// little-endian 16-bit PCM payload extracted from `samples`.
///
/// The resulting `Vec<u8>` is fully self-contained and lives only in
/// RAM. Metadata is hardcoded to 16000 Hz, mono, 16-bit as required
/// by the downstream STT providers.
///
/// Layout (offsets are zero-based):
///   0  "RIFF"            (4 bytes)
///   4  ChunkSize         (u32 LE) = 36 + data length
///   8  "WAVE"            (4 bytes)
///  12  "fmt "            (4 bytes)
///  16  Subchunk1Size     (u32 LE) = 16 (PCM)
///  20  AudioFormat       (u16 LE) = 1 (PCM)
///  22  NumChannels       (u16 LE) = 1
///  24  SampleRate        (u32 LE) = 16000
///  28  ByteRate          (u32 LE) = 32000
///  32  BlockAlign        (u16 LE) = 2
///  34  BitsPerSample     (u16 LE) = 16
///  36  "data"            (4 bytes)
///  40  Subchunk2Size     (u32 LE) = data length
///  44  PCM samples       (i16 LE, interleaved)
pub fn create_wav_buffer(samples: &[i16]) -> Vec<u8> {
    let data_len = samples.len() * (BITS_PER_SAMPLE as usize / 8);
    let chunk_size = 36 + data_len;
    let byte_rate = (TARGET_SAMPLE_RATE * CHANNELS as u32 * BITS_PER_SAMPLE as u32) / 8;
    let block_align = (CHANNELS * BITS_PER_SAMPLE) / 8;

    let mut buffer: Vec<u8> = Vec::with_capacity(44 + data_len);

    // --- RIFF header ------------------------------------------------
    buffer.extend_from_slice(b"RIFF");
    buffer.extend_from_slice(&(chunk_size as u32).to_le_bytes());
    buffer.extend_from_slice(b"WAVE");

    // --- fmt sub-chunk ---------------------------------------------
    buffer.extend_from_slice(b"fmt ");
    buffer.extend_from_slice(&16u32.to_le_bytes()); // PCM subchunk size
    buffer.extend_from_slice(&1u16.to_le_bytes()); // PCM audio format
    buffer.extend_from_slice(&CHANNELS.to_le_bytes());
    buffer.extend_from_slice(&TARGET_SAMPLE_RATE.to_le_bytes());
    buffer.extend_from_slice(&byte_rate.to_le_bytes());
    buffer.extend_from_slice(&block_align.to_le_bytes());
    buffer.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());

    // --- data sub-chunk ---------------------------------------------
    buffer.extend_from_slice(b"data");
    buffer.extend_from_slice(&(data_len as u32).to_le_bytes());

    // --- PCM payload (i16 -> LE bytes) -----------------------------
    for &sample in samples {
        buffer.extend_from_slice(&sample.to_le_bytes());
    }

    buffer
}

/// Acquires the default input device and opens a capture stream at the
/// device's **native** sample rate and channel count. Forcing a non-native
/// config (e.g. 16 kHz mono) on Windows WASAPI shared mode silently fails
/// or returns an error, which was the root cause of transcription never
/// working. Samples are down-mixed to mono `i16` inside the data callback
/// and the native rate is stored in `state.capture_rate` so `stop_capture`
/// can resample to 16 kHz when building the final WAV.
pub fn start_capture(
    state: &Arc<AppState>,
    generation: u64,
    session_id: &str,
) -> Result<(), AudioError> {
    state.clear_audio_buffer();
    let started_at_ms = crate::pipeline_run::epoch_ms();
    let context_preferences = state.context_preferences.read().clone();
    let context = crate::context::capture(&context_preferences);
    let global_formatting_level = *state.formatting_level.read();
    let destination = *state.dictation_destination.read();
    let profiles = state.output_profiles.read().clone();
    let temporary_override = state.temporary_profile_override.read().clone();
    let mut profile = crate::output_policy::resolve_output_profile(
        &profiles,
        &context,
        temporary_override.as_deref(),
        global_formatting_level,
    );
    profile.allow_context_to_cloud &= context_preferences.allow_context_to_cloud;
    let formatting_level = profile.formatting_level;
    let pending_session = crate::pipeline_run::RecordingSession {
        id: session_id.to_string(),
        started_at_ms,
        delivery_target: crate::context::ForegroundTarget::from_snapshot(&context),
        context,
        profile,
        formatting_level,
        destination,
    };

    let host = cpal::default_host();
    let configured_device = crate::settings::load_input_device();
    let device = if let Some(ref name) = configured_device {
        let mut found = None;
        if let Ok(mut devices) = host.input_devices() {
            if let Some(d) = devices.find(|d| d.name().map(|n| &n == name).unwrap_or(false)) {
                found = Some(d);
            }
        }
        found
    } else {
        host.default_input_device()
    };

    let device = device.ok_or(AudioError::NoInputDevice)?;

    log::info!(
        "audio: opened input device {:?}",
        device.name().unwrap_or_else(|_| "<unknown>".into())
    );

    let supported = device
        .default_input_config()
        .map_err(|e| AudioError::DeviceOpen(e.to_string()))?;

    let sample_format = supported.sample_format();
    let native_rate = supported.sample_rate().0;
    let native_channels = supported.channels();

    log::info!(
        "audio: native format {} Hz, {} ch, {:?}",
        native_rate,
        native_channels,
        sample_format
    );

    // Persist so the WAV resampler knows the source rate later.
    *state.capture_rate.write() = native_rate;

    // Use the device's own config — no forced rate or channel count.
    let config: StreamConfig = supported.into();

    let directory = crate::audio_store::default_directory()
        .ok_or_else(|| AudioError::DeviceOpen("Armazenamento de recuperação indisponível".into()))?
        .join("recovery");
    *state.capture_spool.lock() = Some(
        crate::capture_spool::CaptureSpool::start(directory, session_id, native_rate)
            .map_err(AudioError::DeviceOpen)?,
    );
    *state.capture_fault.lock() = None;
    let error_state = state.clone();
    let err_callback = move |_err: cpal::StreamError| {
        *error_state.capture_fault.lock() = Some(
            "O microfone foi desconectado ou o driver falhou. O áudio parcial foi preservado."
                .into(),
        );
    };

    let ch = native_channels as usize;

    let stream: Stream = match sample_format {
        SampleFormat::I16 => {
            let st = state.clone();
            device
                .build_input_stream(
                    &config,
                    move |samples: &[i16], _: &cpal::InputCallbackInfo| {
                        if !st.is_recording() {
                            return;
                        }
                        let mono = downmix_i16(samples, ch);
                        // Never discard a capture block. The stream is stopped
                        // before the buffer is drained, and the level meter only
                        // holds this lock long enough to copy a small window.
                        record_capture_block(&st, &mono);
                    },
                    err_callback,
                    None,
                )
                .map_err(|e| AudioError::StreamBuild(e.to_string()))?
        }
        SampleFormat::F32 => {
            let st = state.clone();
            device
                .build_input_stream(
                    &config,
                    move |samples: &[f32], _: &cpal::InputCallbackInfo| {
                        if !st.is_recording() {
                            return;
                        }
                        let mono = downmix_f32(samples, ch);
                        record_capture_block(&st, &mono);
                    },
                    err_callback,
                    None,
                )
                .map_err(|e| AudioError::StreamBuild(e.to_string()))?
        }
        other => {
            return Err(AudioError::UnsupportedSampleFormat(other));
        }
    };

    stream
        .play()
        .map_err(|e| AudioError::StreamPlay(e.to_string()))?;

    state
        .install_recording_capture(generation, stream, pending_session)
        .map_err(|_| AudioError::Superseded)?;

    // Keep capture local until the silence check at stop. The legacy streaming
    // transport opens only after audio has been admitted.

    Ok(())
}

fn downmix_i16(samples: &[i16], ch: usize) -> Vec<i16> {
    if ch <= 1 {
        return samples.to_vec();
    }
    let mut mono = Vec::with_capacity(samples.len() / ch);
    for frame in samples.chunks(ch) {
        let sum: i32 = frame.iter().map(|&s| s as i32).sum();
        mono.push((sum / ch as i32) as i16);
    }
    mono
}

fn downmix_f32(samples: &[f32], ch: usize) -> Vec<i16> {
    if ch <= 1 {
        return samples
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
    }
    let mut mono = Vec::with_capacity(samples.len() / ch);
    for frame in samples.chunks(ch) {
        let sum: f32 = frame.iter().sum();
        let avg = (sum / ch as f32).clamp(-1.0, 1.0);
        mono.push((avg * 32767.0) as i16);
    }
    mono
}

/// Stops the active capture stream, builds the WAV, runs the selected product
/// mode ([`crate::transcription`]), then delivers the text and records history.
pub async fn stop_capture(
    state: &Arc<AppState>,
    delivery_target: crate::context::ForegroundTarget,
) -> Option<String> {
    if !state.set_recording_delivery_target(delivery_target) {
        log::warn!("audio: stop target captured before recording session became available");
    }
    emit_transcribing(state, true);

    struct TranscribingGuard(Arc<AppState>);
    impl Drop for TranscribingGuard {
        fn drop(&mut self) {
            emit_transcribing(&self.0, false);
        }
    }
    let _guard = TranscribingGuard(state.clone());

    stop_capture_inner(state).await
}

async fn stop_capture_inner(state: &Arc<AppState>) -> Option<String> {
    struct RecordingSessionGuard(Arc<AppState>);
    impl Drop for RecordingSessionGuard {
        fn drop(&mut self) {
            self.0.recording_session.lock().take();
        }
    }
    let _recording_session_guard = RecordingSessionGuard(state.clone());
    let voice_command = state.command_request.lock().take();
    let _ = state.drop_audio_stream();

    let spool = state.capture_spool.lock().take();
    let mut captured_path = None;
    let raw_samples = if let Some(spool) = spool {
        match spool.finish().and_then(|path| {
            let samples = crate::capture_spool::read_pcm(&path)?;
            captured_path = Some(path);
            Ok(samples)
        }) {
            Ok(samples) => samples,
            Err(error) => {
                log::error!("capture: {error}");
                return None;
            }
        }
    } else {
        state.drain_audio_buffer()
    };
    state.clear_audio_buffer();
    let capture_rate = *state.capture_rate.read();
    if crate::speech_presence::is_clearly_silent(&raw_samples, capture_rate) {
        // Only this just-finished empty capture is disposable. Never sweep
        // previous recovery audio or user history when reporting silence.
        if let Some(path) = captured_path {
            if let Err(error) = std::fs::remove_file(path) {
                log::warn!("audio: could not retire empty capture: {error}");
            }
        }
        if let Some(handle) = state.app_handle.read().as_ref() {
            if let Some(gadget) = handle.get_webview_window("gadget") {
                let _ = gadget.emit("recording-no-speech", state.recording_status());
            }
        }
        log::info!("audio: locally skipped silent microphone capture");
        return None;
    }

    let raw_count = raw_samples.len();
    let duration_ms = (raw_count as u64 * 1000) / capture_rate as u64;

    let audio_prepare_started = std::time::Instant::now();
    let resampled_samples = if capture_rate == TARGET_SAMPLE_RATE {
        raw_samples
    } else {
        resample(&raw_samples, capture_rate, TARGET_SAMPLE_RATE)
    };

    let (samples, processing) =
        crate::audio_processing::enhance_microphone_audio(&resampled_samples, TARGET_SAMPLE_RATE);
    if processing.applied {
        log::info!(
            "audio: noise-aware gain applied ({:+.1} dB, active {:.1}->{:.1} dBFS, peak {:.1}->{:.1} dBFS, noise floor {:.1} dBFS, active {:.1}%)",
            processing.gain_db,
            processing.active_rms_before_dbfs,
            processing.active_rms_after_dbfs,
            processing.peak_before_dbfs,
            processing.peak_after_dbfs,
            processing.noise_floor_dbfs,
            processing.active_frame_percent,
        );
    } else {
        log::info!(
            "audio: noise-aware gain skipped (noise floor {:.1} dBFS, speech threshold {:.1} dBFS, active {:.1}%, peak {:.1} dBFS)",
            processing.noise_floor_dbfs,
            processing.speech_threshold_dbfs,
            processing.active_frame_percent,
            processing.peak_before_dbfs,
        );
    }

    let original_wav = processing
        .applied
        .then(|| create_wav_buffer(&resampled_samples));
    let wav = create_wav_buffer(&samples);
    log::info!(
        "audio: WAV buffer generated in RAM ({} samples @ {} Hz -> {} samples @ {} Hz, {} KB)",
        raw_count,
        capture_rate,
        samples.len(),
        TARGET_SAMPLE_RATE,
        wav.len() / 1024
    );

    let id = chrono_like_id();
    crate::pipeline_run::emit_pipeline_progress(
        state,
        crate::pipeline_run::PipelineProgressEvent {
            kind: crate::pipeline_run::PipelineProgressKind::AudioPreparing,
            run_id: Some(id.clone()),
            message: Some("Preparando áudio".into()),
            ..Default::default()
        },
    );
    let audio_path = crate::audio_store::save(&id, "wav", &wav);
    if audio_path.is_some() {
        if let Some(original_wav) = original_wav.as_ref() {
            let _ = crate::audio_store::save_original(&id, "wav", original_wav);
        }
    }
    let audio_prepare_ms = audio_prepare_started.elapsed().as_millis() as u64;

    {
        let mode = *state.transcription_mode.read();
        crate::pipeline_run::emit_pipeline_progress(
            state,
            crate::pipeline_run::PipelineProgressEvent {
                kind: crate::pipeline_run::PipelineProgressKind::Recognizing,
                run_id: Some(id.clone()),
                message: Some("Reconhecendo fala".into()),
                ..Default::default()
            },
        );
        match crate::transcription::run_product_mode_with_duration(
            state,
            wav,
            "audio.wav",
            "audio/wav",
            "wav",
            Some(duration_ms),
        )
        .await
        {
            Ok(mut result) => {
                if result.id.is_empty() {
                    result.id = format!("{id}-run-{}", crate::pipeline_run::epoch_ms());
                }
                result.audio_prepare_ms = Some(audio_prepare_ms);
                result.timings.audio_prepare_ms = Some(audio_prepare_ms);
                result.add_stage(crate::pipeline_run::StageRecord::completed(
                    crate::pipeline_run::StageKind::AudioPrepare,
                    audio_prepare_ms,
                ));
                if let Some(request) = voice_command {
                    if let Err(error) = apply_voice_command(state, &mut result, request).await {
                        let mut entry = crate::transcription::mode_failed_history(
                            id,
                            now_timestamp(),
                            audio_path,
                            duration_ms,
                            "mic",
                            mode,
                            error,
                        );
                        attach_pending_failed_run(state, &mut entry);
                        persist_and_notify(state, &entry);
                        return None;
                    }
                }
                let final_text = result.final_text.clone();
                crate::pipeline_run::emit_pipeline_progress(
                    state,
                    crate::pipeline_run::PipelineProgressEvent {
                        kind: crate::pipeline_run::PipelineProgressKind::Delivering,
                        run_id: Some(id.clone()),
                        message: Some("Entregando texto".into()),
                        ..Default::default()
                    },
                );
                deliver_pipeline_result(state, &mut result).await;
                let entry = crate::transcription::mode_result_to_history(
                    id,
                    now_timestamp(),
                    audio_path,
                    duration_ms,
                    "mic",
                    &result,
                );
                if !persist_and_notify(state, &entry) {
                    return None;
                }
                crate::pipeline_run::emit_pipeline_progress(
                    state,
                    crate::pipeline_run::PipelineProgressEvent {
                        kind: crate::pipeline_run::PipelineProgressKind::Complete,
                        run_id: Some(entry.id.clone()),
                        ..Default::default()
                    },
                );
                log::debug!("audio: mode output ({} chars)", final_text.len());
                Some(final_text)
            }
            Err(err_msg) => {
                crate::pipeline_run::emit_pipeline_progress(
                    state,
                    crate::pipeline_run::PipelineProgressEvent {
                        kind: crate::pipeline_run::PipelineProgressKind::ProviderFailed,
                        run_id: Some(id.clone()),
                        message: Some("A transcrição falhou".into()),
                        ..Default::default()
                    },
                );
                let mut entry = crate::transcription::mode_failed_history(
                    id,
                    now_timestamp(),
                    audio_path,
                    duration_ms,
                    "mic",
                    mode,
                    err_msg,
                );
                attach_pending_failed_run(state, &mut entry);
                persist_and_notify(state, &entry);
                None
            }
        }
    }
}

/// Polyphase windowed-sinc resampling with low-pass filtering before decimation.
pub(crate) fn resample(input: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }
    let cutoff = (to_rate as f64 / from_rate as f64).min(1.0) * 0.90;
    let radius = (24.0 / cutoff).ceil() as isize;
    // Cache trigonometry once per phase. 48k -> 16k needs only one phase.
    let gcd = |mut a: u32, mut b: u32| {
        while b != 0 {
            let r = a % b;
            a = b;
            b = r;
        }
        a
    };
    let phases = (to_rate / gcd(from_rate, to_rate)).min(1024) as usize;
    let kernels: Vec<Vec<f64>> = (0..phases)
        .map(|phase| {
            let fraction = phase as f64 / phases as f64;
            (-radius..=radius)
                .map(|offset| {
                    let distance = offset as f64 - fraction;
                    let x = std::f64::consts::PI * cutoff * distance;
                    let sinc = if x.abs() < 1e-9 { 1.0 } else { x.sin() / x };
                    let window =
                        0.5 + 0.5 * (std::f64::consts::PI * distance / (radius as f64 + 1.0)).cos();
                    cutoff * sinc * window
                })
                .collect()
        })
        .collect();
    let out_len = (input.len() as u64 * to_rate as u64).div_ceil(from_rate as u64) as usize;
    let mut output = Vec::with_capacity(out_len);
    for index in 0..out_len {
        let numerator = index as u64 * from_rate as u64;
        let center = (numerator / to_rate as u64) as isize;
        let phase = ((numerator % to_rate as u64) * phases as u64 / to_rate as u64) as usize;
        let mut sum = 0.0;
        let mut weight_sum = 0.0;
        for (tap, &weight) in kernels[phase].iter().enumerate() {
            let source = center + tap as isize - radius;
            if source >= 0 && (source as usize) < input.len() {
                sum += f64::from(input[source as usize]) * weight;
                weight_sum += weight;
            }
        }
        output.push(
            (sum / weight_sum.max(1e-9))
                .round()
                .clamp(-32768.0, 32767.0) as i16,
        );
    }
    output
}

/// Voice command: the transcript is an instruction. Rewrites the captured
/// selection (or writes new text) and pastes the result over the selection.
async fn apply_voice_command(
    state: &AppState,
    run: &mut crate::pipeline_run::PipelineRun,
    request: crate::models::CommandRequest,
) -> Result<(), String> {
    let instruction = run.final_text.trim().to_string();
    if instruction.is_empty() {
        return Err("Nenhuma instrução foi reconhecida.".into());
    }
    crate::pipeline_run::emit_pipeline_progress(
        state,
        crate::pipeline_run::PipelineProgressEvent {
            kind: crate::pipeline_run::PipelineProgressKind::Refining,
            message: Some("Aplicando comando".into()),
            ..Default::default()
        },
    );
    let out = crate::assist::run_command(state, &instruction, request.selection.as_deref())
        .await
        .map_err(|error| format!("Modo comando: {error}"))?;
    run.final_text = out.text.trim().to_string();
    run.stages.push(format!("voice_command:{}", out.provider));
    run.stages.push(if request.selection.is_some() {
        "voice_command_selection".into()
    } else {
        "voice_command_compose".into()
    });
    run.transcript.formatted = Some(run.final_text.clone());
    run.destination = crate::output_policy::DictationDestination::FocusedField;
    run.delivery.destination = run.destination;
    crate::transcription::modes::record_assist_call(run, "attempt-voice-command", &out);
    Ok(())
}

async fn write_clipboard(
    final_text: &str,
    permit: Option<crate::operations::Permit>,
) -> Result<(), String> {
    let clipboard_text = final_text.to_string();
    let clipboard_fut = tokio::task::spawn_blocking(move || {
        let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        if permit.as_ref().is_some_and(|permit| !permit.valid()) {
            return Err("Entrega cancelada".into());
        }
        clipboard
            .set_text(clipboard_text)
            .map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    });
    match tokio::time::timeout(std::time::Duration::from_secs(3), clipboard_fut).await {
        Ok(Ok(Ok(()))) => {
            log::info!(
                "audio: final text copied to clipboard ({} chars)",
                final_text.len()
            );
            Ok(())
        }
        Ok(Ok(Err(error))) => Err(format!("clipboard write failed: {error}")),
        Ok(Err(error)) => Err(format!("clipboard task failed: {error}")),
        Err(_) => Err("clipboard access timed out after 3s".into()),
    }
}

fn attach_pending_failed_run(state: &AppState, entry: &mut crate::models::HistoryEntry) {
    let Some(mut run) = state.pending_failed_pipeline_run.lock().take() else {
        return;
    };
    if run.id.is_empty() {
        run.id = format!("{}-run-{}", entry.id, crate::pipeline_run::epoch_ms());
    }
    if run.session_id.is_empty() {
        run.session_id = format!("{}-session", entry.id);
    }
    run.normalize();
    entry.pipeline_runs = vec![run];
}

// Compatibility path for the dormant legacy pipeline. Product modes use the
// structured delivery function below and retain its target-safety evidence.
async fn deliver_pipeline_result(state: &AppState, run: &mut crate::pipeline_run::PipelineRun) {
    use crate::output_policy::DictationDestination;
    use crate::pipeline_run::{DeliveryRecord, PipelineError, StageKind, StageRecord};

    let started = std::time::Instant::now();
    let text = run.final_text.clone();
    let expected_target = crate::context::ForegroundTarget {
        hwnd: run.delivery.target_hwnd,
        process_id: run.delivery.target_process_id,
        focus_id: run.delivery.target_focus_id,
    };
    let mut delivery = DeliveryRecord {
        destination: run.destination,
        target_hwnd: expected_target.hwnd,
        target_process_id: expected_target.process_id,
        target_focus_id: expected_target.focus_id,
        delivered_at_ms: Some(crate::pipeline_run::epoch_ms()),
        ..Default::default()
    };
    let outcome = match run.destination {
        DictationDestination::Scratchpad => {
            match crate::scratchpad::add(text.clone(), Some(run.id.clone()), run.profile_id.clone())
            {
                Ok(note) => {
                    delivery.scratchpad_note_id = Some(note.id);
                    Ok(())
                }
                Err(error) => Err(error),
            }
        }
        DictationDestination::ClipboardOnly => {
            let clipboard_started = std::time::Instant::now();
            let result = write_clipboard(&text, state.operations.permit()).await;
            run.timings.clipboard_ms = Some(clipboard_started.elapsed().as_millis() as u64);
            delivery.clipboard_ok = result.is_ok();
            result
        }
        DictationDestination::FocusedField => {
            let clipboard_started = std::time::Instant::now();
            let result = write_clipboard(&text, state.operations.permit()).await;
            run.timings.clipboard_ms = Some(clipboard_started.elapsed().as_millis() as u64);
            delivery.clipboard_ok = result.is_ok();
            result.and_then(|()| {
                if !crate::context::foreground_delivery_target_matches(expected_target) {
                    return Err(
                        "O campo de destino mudou. O texto foi copiado; cole com Ctrl+V.".into(),
                    );
                }
                delivery.paste_attempted = true;
                paste_into_focused_field(expected_target, Some(&state.operations))
                    .map(|()| delivery.paste_ok = true)
            })
        }
    };
    let delivered_somewhere = outcome.is_ok() || delivery.clipboard_ok;
    if let Err(error) = outcome {
        log::warn!("audio: delivery degraded safely: {}", error);
        delivery.error = Some(PipelineError {
            kind: crate::pipeline_run::PipelineErrorKind::Delivery,
            code: "delivery_failed_or_target_changed".into(),
            message: error,
            retryable: true,
        });
    }
    let elapsed = started.elapsed().as_millis() as u64;
    run.timings.delivery_ms = Some(elapsed);
    // Force canonical normalization to recompute total with recognition,
    // transformations, clipboard and delivery rather than preserving the
    // provider-only legacy total.
    run.total_pipeline_ms = None;
    run.timings.total_ms = 0;
    run.transcript.delivered = delivered_somewhere.then_some(text);
    if let Some(clipboard_ms) = run.timings.clipboard_ms {
        let clipboard_stage = if delivery.clipboard_ok {
            StageRecord::completed(StageKind::Clipboard, clipboard_ms)
        } else {
            StageRecord::failed(
                StageKind::Clipboard,
                clipboard_ms,
                PipelineError {
                    kind: crate::pipeline_run::PipelineErrorKind::Clipboard,
                    code: "clipboard_write_failed".into(),
                    message: delivery
                        .error
                        .as_ref()
                        .map(|error| error.message.clone())
                        .unwrap_or_else(|| "clipboard write failed".into()),
                    retryable: true,
                },
            )
        };
        run.add_stage(clipboard_stage);
    }
    let delivery_stage = delivery
        .error
        .clone()
        .map(|error| StageRecord::failed(StageKind::Delivery, elapsed, error))
        .unwrap_or_else(|| StageRecord::completed(StageKind::Delivery, elapsed));
    run.delivery = delivery;
    run.add_stage(delivery_stage);
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoAiEditOutcome {
    ReplacedSelection,
    CopiedToClipboard,
}

/// Restores an immutable transcript version without blindly overwriting an
/// unrelated field. Direct replacement is allowed only when the original
/// window/process still owns focus and the exact delivered text is selected.
pub async fn undo_ai_edit(
    state: &Arc<AppState>,
    entry_id: &str,
    version: &str,
) -> Result<UndoAiEditOutcome, String> {
    let _lease = state.operations.begin("undo")?;
    let entry = crate::history::get(entry_id).ok_or("Histórico não encontrado")?;
    let run = entry
        .pipeline_runs
        .last()
        .ok_or("Pipeline não encontrado")?;
    let replacement = match version {
        "raw" => run.transcript.raw.as_deref(),
        "refined" => run.transcript.refined.as_deref(),
        _ => return Err("Versão de undo inválida".into()),
    }
    .filter(|text| !text.is_empty())
    .ok_or("Versão solicitada não está disponível")?
    .to_string();

    let mut preferences = state.context_preferences.read().clone();
    if let Some(selection) = preferences
        .sources
        .iter_mut()
        .find(|source| source.source == crate::context::ContextSourceKind::Selection)
    {
        selection.enabled = true;
        selection.privacy = crate::context::ContextPrivacy::EphemeralLocal;
    }
    preferences.allow_context_to_cloud = false;
    preferences.persist_raw_context = false;
    let current = crate::context::capture(&preferences);
    let exact_selection = current.selected_text.as_deref() == run.transcript.delivered.as_deref();
    let expected = crate::context::ForegroundTarget {
        hwnd: run.delivery.target_hwnd,
        process_id: run.delivery.target_process_id,
        focus_id: run.delivery.target_focus_id,
    };
    let same_target = crate::context::foreground_delivery_target_matches(expected);
    write_clipboard(&replacement, state.operations.permit()).await?;
    if same_target && exact_selection {
        paste_into_focused_field(expected, Some(&state.operations))?;
        Ok(UndoAiEditOutcome::ReplacedSelection)
    } else {
        Ok(UndoAiEditOutcome::CopiedToClipboard)
    }
}

/// Retries a failed transcription from history using the legacy pipeline.
pub async fn retry_transcription_handler(
    state: &Arc<AppState>,
    id: &str,
) -> Result<String, String> {
    retry_transcription_handler_with_strategy(state, id, false).await
}

pub async fn retry_transcription_handler_with_strategy(
    state: &Arc<AppState>,
    id: &str,
    force_fallback: bool,
) -> Result<String, String> {
    let kind = if crate::history::get(id).is_some_and(|entry| entry.source == "mic") {
        "retry-mic"
    } else {
        "retry-file"
    };
    let lease = state.operations.begin(kind)?;
    tokio::select! { biased;
        _ = lease.cancelled() => Err("Retranscrição cancelada. O áudio foi preservado.".into()),
        result = retry_transcription_handler_with_strategy_inner(state, id, force_fallback) => result,
    }
}
async fn retry_transcription_handler_with_strategy_inner(
    state: &Arc<AppState>,
    id: &str,
    force_fallback: bool,
) -> Result<String, String> {
    emit_transcribing(state, true);

    struct TranscribingGuard(Arc<AppState>);
    impl Drop for TranscribingGuard {
        fn drop(&mut self) {
            emit_transcribing(&self.0, false);
        }
    }
    let _guard = TranscribingGuard(state.clone());

    let entry = crate::history::get(id).ok_or_else(|| "Histórico não encontrado".to_string())?;

    let audio_path = entry
        .audio_path
        .clone()
        .ok_or_else(|| "Este item não possui áudio salvo para retentar".to_string())?;

    let fail =
        |state: &Arc<AppState>, entry: &crate::models::HistoryEntry, msg: String| -> String {
            let failed = crate::transcription::update_failed_entry(state, entry, msg.clone());
            if let Err(error) = persist_retry(state, &failed) {
                return error;
            }
            msg
        };

    let bytes = match crate::audio_store::read(&audio_path) {
        Ok(b) => b,
        Err(e) => {
            let msg = format!("Não foi possível ler o áudio salvo: {}", e);
            return Err(fail(state, &entry, msg));
        }
    };

    let ext = std::path::Path::new(&audio_path)
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wav".to_string());
    let mime = crate::audio_store::mime_for_ext(&ext);

    log::info!("audio: retrying transcription for {}", id);

    {
        let mode = *state.transcription_mode.read();
        struct RetrySessionGuard(Arc<AppState>, bool);
        impl Drop for RetrySessionGuard {
            fn drop(&mut self) {
                if self.1 {
                    self.0.recording_session.lock().take();
                }
            }
        }
        let installed_session = if state.recording_session.lock().is_none() {
            let context = entry
                .pipeline_runs
                .last()
                .map(|run| run.context.clone())
                .unwrap_or_default();
            let profiles = state.output_profiles.read().clone();
            let level = *state.formatting_level.read();
            let override_id = state.temporary_profile_override.read().clone();
            let profile = crate::output_policy::resolve_output_profile(
                &profiles,
                &context,
                override_id.as_deref(),
                level,
            );
            *state.recording_session.lock() = Some(crate::pipeline_run::RecordingSession {
                id: format!("retry-session-{}", crate::pipeline_run::epoch_ms()),
                started_at_ms: crate::pipeline_run::epoch_ms(),
                delivery_target: crate::context::ForegroundTarget::from_snapshot(&context),
                context,
                formatting_level: profile.formatting_level,
                profile,
                destination: *state.dictation_destination.read(),
            });
            true
        } else {
            false
        };
        let _retry_session_guard = RetrySessionGuard(state.clone(), installed_session);
        let run = if force_fallback {
            run_forced_fallback(state, &entry, bytes, mime).await
        } else {
            crate::transcription::run_product_mode_with_duration(
                state,
                bytes,
                "audio.wav",
                mime,
                &ext,
                None,
            )
            .await
        };
        match run {
            Ok(mut result) => {
                if result.id.is_empty() {
                    result.id = format!("{}-run-{}", entry.id, crate::pipeline_run::epoch_ms());
                }
                let final_text = result.final_text.clone();
                if entry.source == "mic" {
                    if let Some(previous) = entry.pipeline_runs.last() {
                        result.context = previous.context.clone();
                        result.destination = previous.destination;
                    }
                    deliver_pipeline_result(state, &mut result).await;
                }
                let updated = crate::transcription::mode_result_to_history(
                    entry.id.clone(),
                    entry.date.clone(),
                    Some(audio_path),
                    entry.duration_ms,
                    &entry.source,
                    &result,
                );
                persist_retry(state, &updated)?;
                Ok(final_text)
            }
            Err(msg) => {
                let mut failed = crate::transcription::mode_failed_history(
                    entry.id.clone(),
                    entry.date.clone(),
                    entry.audio_path.clone(),
                    entry.duration_ms,
                    &entry.source,
                    mode,
                    msg.clone(),
                );
                attach_pending_failed_run(state, &mut failed);
                persist_retry(state, &failed)?;
                Err(msg)
            }
        }
    }
}

async fn run_forced_fallback(
    state: &Arc<AppState>,
    entry: &crate::models::HistoryEntry,
    bytes: Vec<u8>,
    mime: &str,
) -> Result<crate::pipeline_run::PipelineRun, String> {
    use crate::pipeline_run::{
        AttemptResultMetadata, AttemptStatus, AudioTransport, FallbackRecord, ProviderAttempt,
    };
    let prior = entry.pipeline_runs.last();
    let already_used = prior
        .into_iter()
        .flat_map(|run| run.attempts.iter())
        .map(|attempt| attempt.provider.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>();
    let _ = already_used;
    let (provider, model, transport) =
        ("groq", "whisper-large-v3-turbo", AudioTransport::Multipart);
    crate::pipeline_run::emit_pipeline_progress(
        state,
        crate::pipeline_run::PipelineProgressEvent {
            kind: crate::pipeline_run::PipelineProgressKind::FallbackStarted,
            provider: prior
                .and_then(|run| run.attempts.last())
                .map(|attempt| attempt.provider.clone()),
            fallback_provider: Some(provider.into()),
            message: Some(format!("Usando fallback {provider}")),
            ..Default::default()
        },
    );
    let started_at_ms = crate::pipeline_run::epoch_ms();
    let started = std::time::Instant::now();
    let text_result =
        crate::transcription::transcribe_groq_whisper(state, bytes, "audio.wav", mime).await;
    let duration_ms = started.elapsed().as_millis() as u64;
    let text = match text_result {
        Ok(text) => text,
        Err(error) => {
            let mut failed = crate::pipeline_run::PipelineRun::hard_error(
                "",
                prior
                    .map(|run| run.mode)
                    .unwrap_or(*state.transcription_mode.read()),
                error.clone(),
            );
            failed.attempts.push(ProviderAttempt {
                id: format!("forced-{provider}-{started_at_ms}"),
                provider: provider.into(),
                model: model.into(),
                transport,
                started_at_ms,
                duration_ms: Some(duration_ms),
                status: AttemptStatus::Failed,
                error: Some(crate::pipeline_run::PipelineError {
                    kind: crate::pipeline_run::PipelineErrorKind::Provider,
                    code: "manual_fallback_failed".into(),
                    message: error.clone(),
                    retryable: true,
                }),
                ..Default::default()
            });
            failed.fallback = FallbackRecord {
                used: true,
                forced: true,
                reason: Some("manual_fallback_failed".into()),
                from_provider: prior
                    .and_then(|run| run.attempts.last())
                    .map(|attempt| attempt.provider.clone()),
                to_provider: Some(provider.into()),
            };
            *state.pending_failed_pipeline_run.lock() = Some(failed);
            return Err(error);
        }
    };
    let mut run = crate::pipeline_run::PipelineRun {
        mode: prior
            .map(|run| run.mode)
            .unwrap_or(*state.transcription_mode.read()),
        final_text: text.clone(),
        whisper_text: Some(text.clone()),
        model: model.into(),
        history_engine_label: format!("forced-fallback/{provider}"),
        attempts: vec![ProviderAttempt {
            id: format!("forced-{provider}-{started_at_ms}"),
            provider: provider.into(),
            model: model.into(),
            transport,
            started_at_ms,
            duration_ms: Some(duration_ms),
            status: AttemptStatus::Success,
            result: AttemptResultMetadata {
                output_chars: Some(text.len()),
                ..Default::default()
            },
            ..Default::default()
        }],
        fallback: FallbackRecord {
            used: true,
            forced: true,
            reason: Some("manual_fallback".into()),
            from_provider: prior
                .and_then(|run| run.attempts.last())
                .map(|attempt| attempt.provider.clone()),
            to_provider: Some(provider.into()),
        },
        used_fallback: true,
        fallback_reason: Some("manual_fallback".into()),
        transcription_latency_ms: duration_ms,
        ..Default::default()
    };
    if let Some(prior) = prior {
        run.context = prior.context.clone();
        run.profile_id = prior.profile_id.clone();
        run.formatting_level = prior.formatting_level;
        run.destination = prior.destination;
    }
    Ok(crate::transcription::modes::finalize_product_result(
        state, run,
    ))
}

/// Simulates the OS paste shortcut (`Ctrl+V`, or `Cmd+V` on macOS) so the
/// freshly-copied transcription is inserted into whichever input field
/// currently holds focus — the search bar, text editor or chat box the user
/// was typing in when they fired the recording shortcut.
///
/// This is best-effort: if the keystroke simulation fails the text is still on
/// the clipboard, so the user can paste it manually. A short delay gives the
/// foreground application a moment to settle and ensures the clipboard write is
/// visible to it before the paste is dispatched.
fn paste_into_focused_field(
    expected: crate::context::ForegroundTarget,
    coordinator: Option<&crate::operations::Coordinator>,
) -> Result<(), String> {
    use enigo::{
        Direction::{Click, Press, Release},
        Enigo, Key, Keyboard, Settings,
    };

    // Give the target window time to (re)claim focus and the clipboard write
    // time to propagate before we inject the paste keystroke.
    std::thread::sleep(std::time::Duration::from_millis(150));

    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| format!("failed to init input simulator: {}", e))?;

    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    #[cfg(target_os = "windows")]
    let v_key = Key::V;
    #[cfg(not(target_os = "windows"))]
    let v_key = Key::Unicode('v');

    if coordinator.is_some_and(|coordinator| coordinator.status().is_some_and(|job| job.cancelled))
    {
        return Err("Entrega cancelada".into());
    }
    if !crate::context::foreground_delivery_target_matches(expected) {
        return Err("O campo de destino mudou. O texto está na área de transferência.".into());
    }
    enigo
        .key(modifier, Press)
        .map_err(|e| format!("failed to press modifier: {}", e))?;
    let press_v = enigo.key(v_key, Click);
    // Always release the modifier, even if the 'v' press failed, so we never
    // leave Ctrl/Cmd stuck down for the user.
    let release = enigo.key(modifier, Release);

    press_v.map_err(|e| format!("failed to press v: {}", e))?;
    release.map_err(|e| format!("failed to release modifier: {}", e))?;

    log::info!("audio: pasted transcription into focused field");
    Ok(())
}

/// Reads a local audio file and runs the legacy transcription pipeline.
/// Unlike microphone capture, an upload does not hijack the clipboard.
pub async fn transcribe_file_path(state: &Arc<AppState>, path: String) -> Result<String, String> {
    let lease = state.operations.begin("upload")?;
    tokio::select! { biased;
        _ = lease.cancelled() => Err("Transcrição cancelada. O áudio de origem permanece disponível.".into()),
        result = transcribe_file_path_inner(state, path) => result,
    }
}
pub(crate) async fn transcribe_file_path_inner(
    state: &Arc<AppState>,
    path: String,
) -> Result<String, String> {
    emit_transcribing(state, true);

    struct TranscribingGuard(Arc<AppState>);
    impl Drop for TranscribingGuard {
        fn drop(&mut self) {
            emit_transcribing(&self.0, false);
        }
    }
    let _guard = TranscribingGuard(state.clone());

    const MAX_AUDIO_FILE_SIZE: u64 = 50 * 1024 * 1024;
    let metadata = std::fs::metadata(&path)
        .map_err(|e| format!("não foi possível acessar o arquivo: {}", e))?;
    if metadata.len() > MAX_AUDIO_FILE_SIZE {
        return Err(format!(
            "arquivo muito grande ({} MB). O tamanho máximo é {} MB.",
            metadata.len() / 1024 / 1024,
            MAX_AUDIO_FILE_SIZE / 1024 / 1024
        ));
    }

    let bytes =
        std::fs::read(&path).map_err(|e| format!("não foi possível ler o arquivo: {}", e))?;
    if bytes.is_empty() {
        return Err("o arquivo de áudio está vazio".to_string());
    }

    let p = std::path::Path::new(&path);
    let file_name = p
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "upload.wav".to_string());
    let ext = p
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wav".to_string());
    let mime = crate::audio_store::mime_for_ext(&ext);

    log::info!("audio: upload '{}' ({})", file_name, mime);

    let id = chrono_like_id();
    let audio_path = crate::audio_store::save(&id, &ext, &bytes);

    {
        let mode = *state.transcription_mode.read();
        match crate::transcription::run_product_mode_with_duration(
            state, bytes, &file_name, mime, &ext, None,
        )
        .await
        {
            Ok(result) => {
                let final_text = result.final_text.clone();
                let entry = crate::transcription::mode_result_to_history(
                    id,
                    now_timestamp(),
                    audio_path,
                    0,
                    "file",
                    &result,
                );
                if !persist_and_notify(state, &entry) {
                    return Err("Falha ao salvar; áudio preservado para recuperação".into());
                }
                Ok(final_text)
            }
            Err(err_msg) => {
                let full_msg = format!(
                    "Falha na transcrição do arquivo {:?}: {}",
                    file_name, err_msg
                );
                let mut entry = crate::transcription::mode_failed_history(
                    id,
                    now_timestamp(),
                    audio_path,
                    0,
                    "file",
                    mode,
                    full_msg.clone(),
                );
                attach_pending_failed_run(state, &mut entry);
                persist_and_notify(state, &entry);
                Err(full_msg)
            }
        }
    }
}

/// Produces a reasonably-unique id string from the current system time
/// (UTC milliseconds). Avoids pulling in a full UUID crate for a value
/// that only needs to be unique within a single user's history file.
fn chrono_like_id() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", dur.as_millis())
}

/// Returns the current local time formatted as `YYYY-MM-DD HH:MM`.
/// Local wall-clock label `YYYY-MM-DD HH:MM` (not UTC).
fn now_timestamp() -> String {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::GetLocalTime;
        // windows 0.58: GetLocalTime() -> SYSTEMTIME
        let st = unsafe { GetLocalTime() };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute
        )
    }
    #[cfg(not(windows))]
    {
        // Fallback: UTC civil date (non-Windows builds are secondary).
        let dur = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = dur.as_secs() as i64;
        let days = secs / 86400;
        let secs_of_day = secs % 86400;
        let h = secs_of_day / 3600;
        let m = (secs_of_day % 3600) / 60;
        let z = days + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = (z - era * 146097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = if month <= 2 { y + 1 } else { y };
        format!("{:04}-{:02}-{:02} {:02}:{:02}", year, month, d, h, m)
    }
}

/// Hard-stops the active stream and discards every accumulated sample
/// without building a WAV buffer. Used by the panic shortcut.
pub fn cancel_capture(state: &Arc<AppState>) {
    let _ = state.drop_audio_stream();
    state.clear_audio_buffer();
    state.capture_spool.lock().take();
    state.recording_session.lock().take();
    state.command_request.lock().take();
    log::info!("audio: capture cancelled, buffers released");
}

/// Creates a temporary cpal::Stream that reads from the configured input
/// device, calculates RMS level, and emits throttled `"mic-test-level"` events.
///
/// Levels are **coalesced to ≤ ~12.5 Hz** (same cadence as the live gadget
/// waveform). Emitting on every WASAPI buffer (~10 ms) without throttle was
/// saturating the main WebView IPC/React path while "Testar microfone" ran.
pub fn start_mic_test_stream(app: &tauri::AppHandle, state: &Arc<AppState>) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::StreamConfig;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering as AtomicOrdering};

    let host = cpal::default_host();
    let configured_device = crate::settings::load_input_device();

    let device = if let Some(ref name) = configured_device {
        let mut found = None;
        if let Ok(mut devices) = host.input_devices() {
            if let Some(d) = devices.find(|d| d.name().map(|n| &n == name).unwrap_or(false)) {
                found = Some(d);
            }
        }
        found
    } else {
        host.default_input_device()
    };

    let device = device.ok_or_else(|| "Nenhum dispositivo de entrada encontrado".to_string())?;

    log::info!(
        "audio test: opening input device {:?}",
        device.name().unwrap_or_else(|_| "<unknown>".into())
    );

    let supported = device
        .default_input_config()
        .map_err(|e| format!("Falha ao abrir dispositivo: {}", e))?;

    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();

    // Shared throttle state across format-specific callbacks.
    let last_emit_ms = Arc::new(AtomicU64::new(0));
    let last_level_bits = Arc::new(AtomicU32::new(0f32.to_bits()));

    let make_emit = |app: tauri::AppHandle| {
        let last_emit_ms = last_emit_ms.clone();
        let last_level_bits = last_level_bits.clone();
        move |level: f32| {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let prev_ms = last_emit_ms.load(AtomicOrdering::Relaxed);
            let elapsed = now_ms.saturating_sub(prev_ms);
            // Hard cap: at most one emit every 80 ms.
            if elapsed < 80 {
                return;
            }
            let prev_level = f32::from_bits(last_level_bits.load(AtomicOrdering::Relaxed));
            // Skip tiny level noise unless enough time passed for a keep-alive.
            if (level - prev_level).abs() < 0.015 && elapsed < 200 {
                return;
            }
            last_emit_ms.store(now_ms, AtomicOrdering::Relaxed);
            last_level_bits.store(level.to_bits(), AtomicOrdering::Relaxed);
            use tauri::{Emitter, Manager};
            // Prefer the main settings window; fall back to broadcast.
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.emit("mic-test-level", level);
            } else {
                let _ = app.emit("mic-test-level", level);
            }
        }
    };

    let err_callback = |err: cpal::StreamError| {
        log::error!("audio test: stream error: {}", err);
    };

    let stream = match sample_format {
        cpal::SampleFormat::I16 => {
            let emit = make_emit(app.clone());
            device.build_input_stream(
                &config,
                move |samples: &[i16], _: &cpal::InputCallbackInfo| {
                    if samples.is_empty() {
                        return;
                    }
                    let sum_sq: f64 = samples
                        .iter()
                        .map(|&s| {
                            let f = s as f64 / 32768.0;
                            f * f
                        })
                        .sum();
                    let rms = (sum_sq / samples.len() as f64).sqrt();
                    let level = ((rms * 3.2) as f32).clamp(0.0, 1.0);
                    emit(level);
                },
                err_callback,
                None,
            )
        }
        cpal::SampleFormat::F32 => {
            let emit = make_emit(app.clone());
            device.build_input_stream(
                &config,
                move |samples: &[f32], _: &cpal::InputCallbackInfo| {
                    if samples.is_empty() {
                        return;
                    }
                    let sum_sq: f64 = samples.iter().map(|&s| s as f64 * s as f64).sum();
                    let rms = (sum_sq / samples.len() as f64).sqrt();
                    let level = ((rms * 3.2) as f32).clamp(0.0, 1.0);
                    emit(level);
                },
                err_callback,
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let emit = make_emit(app.clone());
            device.build_input_stream(
                &config,
                move |samples: &[u16], _: &cpal::InputCallbackInfo| {
                    if samples.is_empty() {
                        return;
                    }
                    let sum_sq: f64 = samples
                        .iter()
                        .map(|&s| {
                            let f = (s as f64 - 32768.0) / 32768.0;
                            f * f
                        })
                        .sum();
                    let rms = (sum_sq / samples.len() as f64).sqrt();
                    let level = ((rms * 3.2) as f32).clamp(0.0, 1.0);
                    emit(level);
                },
                err_callback,
                None,
            )
        }
        other => {
            return Err(format!(
                "Formato de áudio não suportado pelo dispositivo: {:?}",
                other
            ))
        }
    }
    .map_err(|e| format!("Falha ao criar stream de teste de microfone: {}", e))?;

    stream
        .play()
        .map_err(|e| format!("Falha ao iniciar stream de teste de microfone: {}", e))?;
    *state.test_stream.lock() = Some(stream);

    Ok(())
}

fn persist_retry(state: &Arc<AppState>, entry: &crate::models::HistoryEntry) -> Result<(), String> {
    if state.operations.status().is_some_and(|job| job.cancelled) {
        return Err("Operação cancelada".into());
    }
    if !crate::history::update_entry(entry.clone()) {
        return Err("Não foi possível atualizar o histórico. O áudio e a entrada anterior foram preservados.".into());
    }
    crate::transcription::emit_saved(state, entry);
    Ok(())
}

fn persist_and_notify(state: &Arc<AppState>, entry: &crate::models::HistoryEntry) -> bool {
    if state.operations.status().is_some_and(|job| job.cancelled) {
        return false;
    }
    match crate::history::push(entry.clone()) {
        Ok(()) => {
            if let Some(run) = entry.pipeline_runs.last() {
                crate::maintenance::mark_capture_complete(&run.session_id);
            }
            crate::transcription::emit_saved(state, entry);
            true
        }
        Err(error) => {
            log::error!("history: {error}");
            if let Some(app) = state.app_handle.read().as_ref() {
                use tauri::Emitter;
                let _ = app.emit("storage-error", "Não foi possível salvar a transcrição. O áudio foi preservado; verifique o armazenamento e tente recuperar o ditado.");
            }
            false
        }
    }
}

fn record_capture_block(state: &Arc<AppState>, samples: &[i16]) {
    if let Some(spool) = state.capture_spool.lock().as_ref() {
        if let Err(error) = spool.push(samples) {
            *state.capture_fault.lock() = Some(error);
        }
    }
    let mut meter = state.audio_buffer.lock();
    let keep = 16_384usize;
    if samples.len() >= keep {
        meter.clear();
        meter.extend_from_slice(&samples[samples.len() - keep..]);
    } else {
        let excess = (meter.len() + samples.len()).saturating_sub(keep);
        meter.drain(..excess);
        meter.extend_from_slice(samples);
    }
}

#[cfg(test)]
mod signal_tests {
    use super::*;
    #[test]
    fn lowpass_preserves_voice_band_and_rejects_aliases() {
        let tone = |hz: f64| {
            (0..48000)
                .map(|i| {
                    (12000.0 * (2.0 * std::f64::consts::PI * hz * i as f64 / 48000.0).sin()) as i16
                })
                .collect::<Vec<_>>()
        };
        let rms = |samples: &[i16]| {
            (samples[200..samples.len() - 200]
                .iter()
                .map(|s| (*s as f64).powi(2))
                .sum::<f64>()
                / (samples.len() - 400) as f64)
                .sqrt()
        };
        let voice = resample(&tone(1000.0), 48000, 16000);
        let alias = resample(&tone(12000.0), 48000, 16000);
        assert_eq!(voice.len(), 16000);
        assert!((rms(&voice) - 12000.0 / 2f64.sqrt()).abs() < 100.0);
        assert!(
            rms(&alias) / rms(&voice) < 0.01,
            "at least 40 dB attenuation required"
        );
        assert!(resample(&vec![0; 48000], 48000, 16000)
            .iter()
            .all(|s| *s == 0));
        for rate in [8000, 44100, 96000] {
            assert_eq!(resample(&vec![123; rate], rate as u32, 16000).len(), 16000);
        }
    }
    #[test]
    fn resampling_minute_stays_within_local_processing_budget() {
        let samples = vec![100i16; 48000 * 60];
        let start = std::time::Instant::now();
        let result = resample(&samples, 48000, 16000);
        assert_eq!(result.len(), 16000 * 60);
        assert!(result.iter().all(|s| *s == 100));
        eprintln!("resample 60s 48k->16k: {} ms", start.elapsed().as_millis());
        assert!(start.elapsed().as_secs() < 30);
    }
}
