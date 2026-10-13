//! Keeps the microphone stream open between dictations.
//!
//! Some devices — notably wireless headsets such as the Logitech G733 — deliver
//! exact digital silence for 0.4–2 s after a capture stream is opened while the
//! radio switches the mic path on. Opening the stream only when the shortcut
//! fires therefore cut the first words of every dictation. A ready stream keeps
//! that warm-up out of the way and fills a short in-memory pre-roll, so speech
//! that starts together with the shortcut is still recorded.
//!
//! The single stream slot (`AppState::audio_stream`) holds the open stream both
//! while recording and while idle-but-ready. Only the stream whose id matches
//! `AppState::capture_owner` writes into the spool or the pre-roll, so a stream
//! being opened or retired can never interleave samples into a recording.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::models::{AppState, RecordingPhase};

/// How long the microphone stays open between dictations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MicReadyMode {
    /// Open the device only while recording (previous behavior).
    Off,
    /// Keep it open for [`AFTER_USE_WINDOW`] after the last dictation.
    #[default]
    AfterUse,
    /// Keep it open while Sonora is running.
    Always,
}

/// Idle time after which an `AfterUse` stream is released.
pub const AFTER_USE_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Audio before the shortcut that is prepended to a dictation.
pub const PREROLL_MS: u64 = 400;
/// Audio kept in the idle ring buffer. Larger than [`PREROLL_MS`] so start-up
/// work between the shortcut and the splice never eats into the pre-roll.
const PREROLL_KEEP_MS: u64 = 1_500;
/// A stream without callbacks for this long is treated as dead (sleep/resume,
/// driver reset) and reopened.
const STALL_AFTER: Duration = Duration::from_secs(2);
/// Upper bound for waiting on the first non-silent block before showing the
/// recording cue anyway (a hardware noise gate also outputs exact zeros).
pub const SIGNAL_WAIT: Duration = Duration::from_millis(1_500);
const SUPERVISOR_TICK: Duration = Duration::from_millis(500);
const DEFAULT_DEVICE_CHECK: Duration = Duration::from_secs(5);
const OPEN_RETRY_BACKOFF: Duration = Duration::from_secs(10);

static NEXT_STREAM_ID: AtomicU64 = AtomicU64::new(1);

/// Liveness of one input stream, shared with its audio callbacks.
pub struct StreamHealth {
    pub id: u64,
    opened_at: Instant,
    /// Milliseconds after `opened_at` of the latest callback; `u64::MAX` before
    /// the first one.
    last_block_ms: AtomicU64,
    /// Milliseconds after `opened_at` of the first non-zero sample.
    first_signal_ms: AtomicU64,
    faulted: AtomicBool,
}

impl StreamHealth {
    pub fn new() -> Self {
        Self {
            id: NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed),
            opened_at: Instant::now(),
            last_block_ms: AtomicU64::new(u64::MAX),
            first_signal_ms: AtomicU64::new(u64::MAX),
            faulted: AtomicBool::new(false),
        }
    }

    pub fn mark_block(&self, samples: &[i16]) {
        let now = self.opened_at.elapsed().as_millis() as u64;
        self.last_block_ms.store(now, Ordering::Relaxed);
        if self.first_signal_ms.load(Ordering::Relaxed) == u64::MAX
            && samples.iter().any(|sample| *sample != 0)
        {
            self.first_signal_ms.store(now, Ordering::Relaxed);
        }
    }

    pub fn mark_fault(&self) {
        self.faulted.store(true, Ordering::Relaxed);
    }

    pub fn faulted(&self) -> bool {
        self.faulted.load(Ordering::Relaxed)
    }

    /// Milliseconds after opening at which real (non-zero) audio first arrived.
    pub fn first_signal_ms(&self) -> Option<u64> {
        let value = self.first_signal_ms.load(Ordering::Relaxed);
        (value != u64::MAX).then_some(value)
    }

    /// Whether callbacks are still arriving. A fresh stream gets the stall
    /// window to deliver its first block.
    pub fn alive(&self) -> bool {
        if self.faulted() {
            return false;
        }
        let elapsed = self.opened_at.elapsed();
        match self.last_block_ms.load(Ordering::Relaxed) {
            u64::MAX => elapsed < STALL_AFTER,
            last => elapsed.saturating_sub(Duration::from_millis(last)) < STALL_AFTER,
        }
    }
}

impl Default for StreamHealth {
    fn default() -> Self {
        Self::new()
    }
}

/// An open input stream and what it was opened for.
pub struct InputStream {
    pub stream: cpal::Stream,
    /// Configured device name (`None` = Windows default) at open time.
    pub configured_device: Option<String>,
    /// Actual device name, used to notice a changed Windows default.
    pub device_name: String,
    pub rate: u32,
    pub health: Arc<StreamHealth>,
    pub last_used: Instant,
}

/// Routes one mono capture block. The owning stream writes to the recording
/// spool when one is installed and to the pre-roll ring otherwise; any other
/// stream only reports liveness.
pub fn route_block(state: &AppState, health: &StreamHealth, rate: u32, mono: &[i16]) {
    health.mark_block(mono);
    if state.capture_owner.load(Ordering::Acquire) != health.id {
        return;
    }
    // The spool lock is held while touching the pre-roll so a recording start
    // (spool lock → pre-roll lock) never misses or duplicates a block.
    let spool_slot = state.capture_spool.lock();
    if let Some(spool) = spool_slot.as_ref() {
        if let Err(error) = spool.push(mono) {
            *state.capture_fault.lock() = Some(error);
        }
        drop(spool_slot);
        crate::audio::record_meter_block(state, mono);
    } else {
        let capacity = (rate as u64 * PREROLL_KEEP_MS / 1000) as usize;
        push_ring(&mut state.preroll.lock(), mono, capacity);
    }
}

pub(crate) fn push_ring(ring: &mut VecDeque<i16>, samples: &[i16], capacity: usize) {
    if samples.len() >= capacity {
        ring.clear();
        ring.extend(&samples[samples.len() - capacity..]);
        return;
    }
    let excess = (ring.len() + samples.len()).saturating_sub(capacity);
    ring.drain(..excess);
    ring.extend(samples);
}

pub(crate) fn ring_tail(ring: &VecDeque<i16>, count: usize) -> Vec<i16> {
    let start = ring.len().saturating_sub(count);
    ring.range(start..).copied().collect()
}

/// A recording attached to the ready stream.
pub struct ReadyCapture {
    pub rate: u32,
    pub health: Arc<StreamHealth>,
    pub preroll_ms: u64,
}

/// Installs a spool for a recording on the ready stream, seeded with the audio
/// captured in the `PREROLL_MS` before `requested_at`. Returns `None` (nothing
/// installed) when no healthy ready stream exists for the configured device.
pub fn attach_ready_stream(
    state: &AppState,
    configured_device: &Option<String>,
    requested_at: Instant,
    open_spool: impl FnOnce(u32) -> Result<crate::capture_spool::CaptureSpool, String>,
) -> Option<Result<ReadyCapture, String>> {
    let mut slot = state.audio_stream.lock();
    let ready = slot
        .as_mut()
        .filter(|stream| stream.health.alive() && stream.configured_device == *configured_device)?;
    if state.capture_owner.load(Ordering::Acquire) != ready.health.id {
        return None;
    }
    ready.last_used = Instant::now();
    let rate = ready.rate;
    let health = ready.health.clone();
    drop(slot);

    let spool = match open_spool(rate) {
        Ok(spool) => spool,
        Err(error) => return Some(Err(error)),
    };
    let wanted_ms = PREROLL_MS + requested_at.elapsed().as_millis() as u64;
    let wanted = (rate as u64 * wanted_ms / 1000) as usize;
    let mut spool_slot = state.capture_spool.lock();
    let preroll = {
        let mut ring = state.preroll.lock();
        let tail = ring_tail(&ring, wanted);
        ring.clear();
        tail
    };
    // Bounded chunks stay within the spool's per-block and queue limits.
    for chunk in preroll.chunks(rate as usize) {
        if let Err(error) = spool.push(chunk) {
            return Some(Err(error));
        }
    }
    *spool_slot = Some(spool);
    drop(spool_slot);
    *state.capture_rate.write() = rate;
    let preroll_ms = preroll.len() as u64 * 1000 / rate.max(1) as u64;
    Some(Ok(ReadyCapture {
        rate,
        health,
        preroll_ms,
    }))
}

/// Called when a recording ends or is cancelled: keeps the stream ready or
/// releases the device, depending on the user's choice and stream health.
pub fn release_after_capture(state: &AppState) {
    let keep = state.features.read().mic_ready != MicReadyMode::Off;
    let mut slot = state.audio_stream.lock();
    let keep_open = keep && slot.as_ref().is_some_and(|stream| stream.health.alive());
    let retired = if keep_open {
        if let Some(stream) = slot.as_mut() {
            stream.last_used = Instant::now();
        }
        None
    } else {
        slot.take()
    };
    drop(slot);
    if let Some(stream) = retired {
        retire(state, stream, "released after capture");
    }
}

/// Drops an idle ready stream so the next dictation reopens the device (for
/// example after the input device setting changed).
pub fn invalidate(state: &AppState) {
    let retired = {
        let mut slot = state.audio_stream.lock();
        if state.recording_status().phase != RecordingPhase::Idle {
            return;
        }
        slot.take()
    };
    if let Some(stream) = retired {
        retire(state, stream, "input device changed");
    }
}

fn retire(state: &AppState, stream: InputStream, reason: &str) {
    let _ = state.capture_owner.compare_exchange(
        stream.health.id,
        0,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    state.preroll.lock().clear();
    log::info!("mic_ready: closed {:?} ({reason})", stream.device_name);
    drop(stream);
}

/// Background thread that opens, keeps and releases the ready stream.
pub fn spawn_supervisor(state: Arc<AppState>) {
    std::thread::Builder::new()
        .name("mic-ready".into())
        .spawn(move || {
            let mut last_default_check = Instant::now();
            let mut retry_after: Option<Instant> = None;
            loop {
                std::thread::sleep(SUPERVISOR_TICK);
                if state.recording_status().phase != RecordingPhase::Idle {
                    continue;
                }
                let mode = state.features.read().mic_ready;
                let check_default = last_default_check.elapsed() >= DEFAULT_DEVICE_CHECK;
                if check_default {
                    last_default_check = Instant::now();
                }

                let default_changed = if check_default {
                    default_device_changed(&state)
                } else {
                    None
                };
                // Decide and take under the slot lock, re-checking the phase
                // there: a recording start takes this lock after entering
                // `Starting`, so it can never attach to a stream retired here.
                let retired = {
                    let mut slot = state.audio_stream.lock();
                    let reason = slot.as_ref().and_then(|stream| {
                        if mode == MicReadyMode::Off {
                            Some("disabled")
                        } else if !stream.health.alive() {
                            Some("stream stopped or device lost")
                        } else if mode == MicReadyMode::AfterUse
                            && stream.last_used.elapsed() >= AFTER_USE_WINDOW
                        {
                            Some("idle window elapsed")
                        } else {
                            default_changed
                        }
                    });
                    let idle = state.recording_status().phase == RecordingPhase::Idle;
                    match (reason, idle) {
                        (Some(reason), true) => slot.take().map(|stream| (stream, reason)),
                        _ => None,
                    }
                };
                if let Some((stream, reason)) = retired {
                    retire(&state, stream, reason);
                }

                let missing = state.audio_stream.lock().is_none();
                if mode == MicReadyMode::Always
                    && missing
                    && retry_after.is_none_or(|at| Instant::now() >= at)
                {
                    let configured = crate::settings::load_input_device();
                    match crate::audio::open_input_stream(&state, configured) {
                        Ok(stream) => {
                            retry_after = None;
                            let mut slot = state.audio_stream.lock();
                            if slot.is_none()
                                && state.recording_status().phase == RecordingPhase::Idle
                            {
                                state.preroll.lock().clear();
                                state
                                    .capture_owner
                                    .store(stream.health.id, Ordering::Release);
                                match cpal::traits::StreamTrait::play(&stream.stream) {
                                    Ok(()) => {
                                        log::info!("mic_ready: opened {:?}", stream.device_name);
                                        *slot = Some(stream);
                                    }
                                    Err(error) => {
                                        log::warn!(
                                            "mic_ready: could not start microphone: {error}"
                                        );
                                        retry_after = Some(Instant::now() + OPEN_RETRY_BACKOFF);
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            log::warn!("mic_ready: could not open microphone: {error}");
                            retry_after = Some(Instant::now() + OPEN_RETRY_BACKOFF);
                        }
                    }
                }
            }
        })
        .expect("spawn mic-ready supervisor");
}

/// When following the Windows default device, reports a changed default.
fn default_device_changed(state: &AppState) -> Option<&'static str> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let current = {
        let slot = state.audio_stream.lock();
        let stream = slot.as_ref()?;
        if stream.configured_device.is_some() {
            return None;
        }
        stream.device_name.clone()
    };
    let default = cpal::default_host()
        .default_input_device()
        .and_then(|device| device.name().ok())?;
    (default != current).then_some("Windows default input changed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_only_the_newest_samples() {
        let mut ring = VecDeque::new();
        push_ring(&mut ring, &[1, 2, 3], 5);
        push_ring(&mut ring, &[4, 5, 6, 7], 5);
        assert_eq!(
            ring.iter().copied().collect::<Vec<_>>(),
            vec![3, 4, 5, 6, 7]
        );
        push_ring(&mut ring, &[8, 9, 10, 11, 12, 13], 5);
        assert_eq!(
            ring.iter().copied().collect::<Vec<_>>(),
            vec![9, 10, 11, 12, 13]
        );
    }

    #[test]
    fn tail_is_bounded_by_available_audio() {
        let ring: VecDeque<i16> = (1..=6).collect();
        assert_eq!(ring_tail(&ring, 2), vec![5, 6]);
        assert_eq!(ring_tail(&ring, 50), vec![1, 2, 3, 4, 5, 6]);
        assert!(ring_tail(&VecDeque::new(), 4).is_empty());
    }

    #[test]
    fn first_signal_ignores_digital_silence() {
        let health = StreamHealth::new();
        health.mark_block(&[0; 480]);
        assert_eq!(health.first_signal_ms(), None);
        assert!(health.alive());
        health.mark_block(&[0, 0, 3, 0]);
        assert!(health.first_signal_ms().is_some());
        health.mark_fault();
        assert!(!health.alive());
    }

    #[test]
    fn mode_defaults_to_after_use_and_uses_snake_case() {
        assert_eq!(MicReadyMode::default(), MicReadyMode::AfterUse);
        assert_eq!(
            serde_json::to_string(&MicReadyMode::AfterUse).unwrap(),
            "\"after_use\""
        );
    }
}
