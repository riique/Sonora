//! Conservative two-pass post-capture processing for microphone recordings.
//!
//! Modern neural speech-to-text models (such as AssemblyAI STT-3.7 and
//! Whisper) already handle moderate ambient noise well; aggressive spectral
//! denoising hurts accuracy by distorting unvoiced consonants (/s/, /f/, /t/,
//! /p/, /k/). Instead, this module:
//!
//! 1. Keeps recordings that are already clean and well-leveled bit-identical
//!    to the original capture (zero unnecessary DSP).
//! 2. Uses pause-safe speech/noise estimation so multi-second thinking pauses
//!    never break speech detection or get their room tone amplified.
//! 3. Applies conservative local speech-aware dynamic leveling with lookahead
//!    and hangover margins so quiet voices, loud voices, and sudden volume
//!    changes inside the same recording stay intelligible without chopping
//!    initial consonants or soft Brazilian Portuguese word endings.
//! 4. Protects sudden loud peaks with a smooth soft-knee limiter (never hard
//!    clipping) and removes DC offset / sub-vocal rumble (< 60 Hz) when
//!    processing is applied.

const FRAME_MS: usize = 20;
const TARGET_ACTIVE_DBFS: f64 = -20.0;
const MAX_GAIN_DB: f64 = 14.0;
const MAX_ATTENUATION_DB: f64 = -10.0;
const MIN_USEFUL_GAIN_DB: f64 = 0.5;
const MIN_DYNAMIC_RANGE_DB: f64 = 8.0;
const SPEECH_ABOVE_NOISE_DB: f64 = 7.0;
const MIN_SPEECH_GATE_DBFS: f64 = -56.0;
const MAX_SPEECH_GATE_DBFS: f64 = -24.0;

/// Margins around active speech frames (in 20 ms frames).
/// Lookahead protects initial consonants; hangover protects weak unstressed
/// final syllables and bridges brief inter-word micro-pauses.
const LOOKAHEAD_FRAMES: usize = 6; // 120 ms
const HANGOVER_FRAMES: usize = 14; // 280 ms
/// Local active-speech neighborhood (±200 ms = 400 ms window) for slow dynamic
/// leveling: steady across syllables while responsive to sudden phrase-level
/// volume jumps.
const LOCAL_SPEECH_RADIUS_FRAMES: usize = 10;

/// Healthy range where original audio is passed through untouched.
/// Dynamic spread is measured on the 400 ms local speech envelope (phrase level),
/// so natural vowel-vs-consonant variations inside a healthy sentence do not
/// trigger unnecessary processing.
const OPTIMAL_MIN_ACTIVE_DBFS: f64 = -25.0;
const OPTIMAL_MAX_ACTIVE_DBFS: f64 = -14.0;
const OPTIMAL_MAX_SPREAD_DB: f64 = 12.0;
const OPTIMAL_MAX_PEAK_DBFS: f64 = -1.0;
const OPTIMAL_MAX_DC_DBFS: f64 = -40.0;

/// Soft-knee peak limiter bounds (C1 smooth tanh knee, zero hard clipping).
const LIMITER_THRESHOLD_DBFS: f64 = -3.0;
const LIMITER_CEILING_DBFS: f64 = -1.0;

/// Gentle 1st-order DC-blocking high-pass cutoff (20 Hz) removes DC offset and
/// sub-sonic rumble while leaving all male/female vocal fundamentals (>= 85 Hz)
/// within ~0.2 dB of unity.
const SUBSONIC_HPF_HZ: f64 = 20.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct AudioProcessingStats {
    pub applied: bool,
    pub used_original: bool,
    pub dc_removed: bool,
    pub gain_db: f64,
    pub min_gain_db: f64,
    pub max_gain_db: f64,
    pub noise_floor_dbfs: f64,
    pub speech_threshold_dbfs: f64,
    pub active_rms_before_dbfs: f64,
    pub active_rms_after_dbfs: f64,
    pub dynamic_spread_before_db: f64,
    pub dynamic_spread_after_db: f64,
    pub peak_before_dbfs: f64,
    pub peak_after_dbfs: f64,
    pub active_frame_percent: f64,
}

/// Conservatively levels quiet/loud speech and sudden volume changes while
/// preserving original audio whenever the capture is already well-leveled and
/// never boosting long thinking pauses.
///
/// The returned samples are a new buffer so the captured/resampled input stays
/// available to the caller (e.g. to persist `.original.wav`).
pub fn enhance_microphone_audio(
    samples: &[i16],
    sample_rate: u32,
) -> (Vec<i16>, AudioProcessingStats) {
    if samples.is_empty() || sample_rate == 0 {
        return (
            samples.to_vec(),
            AudioProcessingStats {
                used_original: true,
                ..AudioProcessingStats::default()
            },
        );
    }

    let frame_size = ((sample_rate as usize * FRAME_MS) / 1_000).max(1);
    let frame_count = samples.len().div_ceil(frame_size);
    if frame_count < 3 {
        return (samples.to_vec(), unchanged_stats(samples));
    }

    // True hardware DC offset is present across frames, whereas asymmetric
    // loud vocal waveforms only bias loud frames. Use the median per-frame
    // mean to detect hardware DC, and remove each frame's own mean when
    // measuring frame AC energy so silent pauses are never polluted by loud
    // speech asymmetry.
    let mut frame_means: Vec<f64> = samples.chunks(frame_size).map(mean_linear).collect();
    frame_means.sort_by(f64::total_cmp);
    let dc_offset = percentile(&frame_means, 0.50);
    let dc_offset_dbfs = linear_to_dbfs(dc_offset.abs());
    let has_significant_dc = dc_offset_dbfs >= OPTIMAL_MAX_DC_DBFS;

    let frame_levels: Vec<f64> = samples.chunks(frame_size).map(frame_ac_rms_dbfs).collect();

    let noise_floor_dbfs = estimate_noise_floor(&frame_levels);
    let speech_threshold_dbfs = (noise_floor_dbfs + SPEECH_ABOVE_NOISE_DB)
        .clamp(MIN_SPEECH_GATE_DBFS, MAX_SPEECH_GATE_DBFS);

    let active_frames: Vec<usize> = frame_levels
        .iter()
        .enumerate()
        .filter_map(|(index, &level)| (level >= speech_threshold_dbfs).then_some(index))
        .collect();

    let peak_before = peak_linear(samples);
    let peak_before_dbfs = linear_to_dbfs(peak_before);

    let mut stats = AudioProcessingStats {
        applied: false,
        used_original: true,
        dc_removed: false,
        gain_db: 0.0,
        min_gain_db: 0.0,
        max_gain_db: 0.0,
        noise_floor_dbfs,
        speech_threshold_dbfs,
        active_rms_before_dbfs: -120.0,
        active_rms_after_dbfs: -120.0,
        dynamic_spread_before_db: 0.0,
        dynamic_spread_after_db: 0.0,
        peak_before_dbfs,
        peak_after_dbfs: peak_before_dbfs,
        active_frame_percent: 100.0 * active_frames.len() as f64 / frame_levels.len() as f64,
    };

    // Pause-safe separation check: require at least 2 active frames (40 ms),
    // independent of total recording length, so long thinking pauses never
    // disqualify speech.
    if active_frames.len() < 2 || peak_before <= 0.0 {
        return (samples.to_vec(), stats);
    }

    let mut is_active = vec![false; frame_count];
    for &idx in &active_frames {
        is_active[idx] = true;
    }

    let mut active_levels: Vec<f64> = active_frames.iter().map(|&i| frame_levels[i]).collect();
    active_levels.sort_by(f64::total_cmp);
    let active_p85_dbfs = percentile(&active_levels, 0.85);

    // Measure phrase-level dynamic spread over the 400 ms local speech envelope
    // so natural vowel-vs-consonant contrast inside a healthy sentence does not
    // masquerade as a sudden volume change.
    let mut local_before_levels: Vec<f64> = active_frames
        .iter()
        .map(|&i| {
            local_active_speech_dbfs(
                i,
                &frame_levels,
                &is_active,
                &active_frames,
                LOCAL_SPEECH_RADIUS_FRAMES,
            )
        })
        .collect();
    local_before_levels.sort_by(f64::total_cmp);
    let dynamic_spread_before_db =
        (percentile(&local_before_levels, 0.90) - percentile(&local_before_levels, 0.10)).max(0.0);
    stats.dynamic_spread_before_db = dynamic_spread_before_db;
    stats.dynamic_spread_after_db = dynamic_spread_before_db;

    let separation_is_reliable = active_p85_dbfs - noise_floor_dbfs >= MIN_DYNAMIC_RANGE_DB;

    let active_rms_before = ac_rms_for_frames(samples, frame_size, &active_frames, dc_offset);
    stats.active_rms_before_dbfs = linear_to_dbfs(active_rms_before);
    stats.active_rms_after_dbfs = stats.active_rms_before_dbfs;

    if !separation_is_reliable || active_rms_before <= 0.0 {
        if has_significant_dc {
            let cleaned = apply_subsonic_hpf_i16(samples, sample_rate);
            stats.applied = true;
            stats.used_original = false;
            stats.dc_removed = true;
            stats.peak_after_dbfs = linear_to_dbfs(peak_linear(&cleaned));
            return (cleaned, stats);
        }
        return (samples.to_vec(), stats);
    }

    // Preserve the original audio verbatim when it is already well-leveled,
    // consistent in volume, free of clipping risk, and free of DC offset.
    let already_optimal = stats.active_rms_before_dbfs >= OPTIMAL_MIN_ACTIVE_DBFS
        && stats.active_rms_before_dbfs <= OPTIMAL_MAX_ACTIVE_DBFS
        && dynamic_spread_before_db <= OPTIMAL_MAX_SPREAD_DB
        && peak_before_dbfs <= OPTIMAL_MAX_PEAK_DBFS
        && !has_significant_dc;
    if already_optimal {
        return (samples.to_vec(), stats);
    }

    // Step 1: Gentle sub-sonic high-pass (20 Hz) removes DC offset and rumble
    // without affecting speech fundamentals, formants, or consonants.
    let clean = apply_subsonic_hpf_f64(samples, sample_rate);
    let clean_frame_levels: Vec<f64> = clean.chunks(frame_size).map(rms_dbfs_f64).collect();

    // Step 2: Build speech-protected mask with lookahead + hangover.
    // Frames inside this mask receive smooth speech leveling; frames outside
    // (long thinking pauses) stay at unity gain (0 dB).
    let mut speech_mask = vec![false; frame_count];
    for &idx in &active_frames {
        let start = idx.saturating_sub(LOOKAHEAD_FRAMES);
        let end = (idx + HANGOVER_FRAMES + 1).min(frame_count);
        for flag in &mut speech_mask[start..end] {
            *flag = true;
        }
    }

    // Cap upward gain in noisier rooms so speech bursts do not carry loud hiss.
    let noise_aware_max_gain = if noise_floor_dbfs > -54.0 {
        (-40.0 - noise_floor_dbfs).clamp(3.0, MAX_GAIN_DB)
    } else {
        MAX_GAIN_DB
    };

    // Step 3: Compute conservative local desired gain (dB) per frame.
    let mut desired_gain_db = vec![0.0_f64; frame_count];
    for i in 0..frame_count {
        if !speech_mask[i] {
            desired_gain_db[i] = 0.0;
            continue;
        }
        let local_level = local_active_speech_dbfs(
            i,
            &clean_frame_levels,
            &is_active,
            &active_frames,
            LOCAL_SPEECH_RADIUS_FRAMES,
        );
        let raw_gain = TARGET_ACTIVE_DBFS - local_level;
        desired_gain_db[i] = raw_gain.clamp(MAX_ATTENUATION_DB, noise_aware_max_gain);
    }

    // Step 4: Bidirectional smoothing of the gain curve (dB) so loud onsets
    // are anticipated cleanly and transitions never pump phonemes or click.
    // Backward lookahead pass: anticipate sudden loud voice jumps so gain drops
    // slightly before a loud syllable arrives.
    for i in (0..frame_count.saturating_sub(1)).rev() {
        if desired_gain_db[i] > desired_gain_db[i + 1] + 3.5 {
            desired_gain_db[i] = desired_gain_db[i + 1] + 3.5;
        }
    }
    // Forward smoothing pass: fast downward (reacts to loud bursts), smooth
    // upward (reaches ~95% of target inside the 120 ms lookahead without
    // syllable pumping or clicks).
    let mut smoothed_gain_db = vec![0.0_f64; frame_count];
    smoothed_gain_db[0] = desired_gain_db[0];
    for i in 1..frame_count {
        let target = desired_gain_db[i];
        let prev = smoothed_gain_db[i - 1];
        let alpha = if target < prev { 0.65 } else { 0.42 };
        smoothed_gain_db[i] = prev + (target - prev) * alpha;
    }

    let min_gain_db = smoothed_gain_db
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let max_gain_db = smoothed_gain_db
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let active_mean_gain_db = active_frames
        .iter()
        .map(|&i| smoothed_gain_db[i])
        .sum::<f64>()
        / active_frames.len() as f64;

    let max_abs_change = min_gain_db.abs().max(max_gain_db.abs());
    let needs_limiter = peak_before_dbfs > LIMITER_THRESHOLD_DBFS;
    if max_abs_change < MIN_USEFUL_GAIN_DB && !has_significant_dc && !needs_limiter {
        return (samples.to_vec(), stats);
    }

    // Step 5: Per-sample linear gain interpolation + smooth soft-knee peak limiter.
    let threshold_lin = db_to_linear(LIMITER_THRESHOLD_DBFS);
    let ceiling_lin = db_to_linear(LIMITER_CEILING_DBFS);
    let mut output = Vec::with_capacity(samples.len());

    for (frame_index, frame) in clean.chunks(frame_size).enumerate() {
        let g0 = db_to_linear(smoothed_gain_db[frame_index]);
        let g1 = db_to_linear(smoothed_gain_db[(frame_index + 1).min(frame_count - 1)]);
        let len = frame.len().max(1) as f64;
        for (sample_idx, &sample) in frame.iter().enumerate() {
            let t = sample_idx as f64 / len;
            let gain = g0 + (g1 - g0) * t;
            let scaled = sample * gain;
            let limited = soft_knee_limit(scaled, threshold_lin, ceiling_lin);
            output.push((limited * 32768.0).round().clamp(-32768.0, 32767.0) as i16);
        }
    }

    let output_frame_levels: Vec<f64> = output.chunks(frame_size).map(rms_dbfs).collect();
    let mut local_after_levels: Vec<f64> = active_frames
        .iter()
        .map(|&i| {
            local_active_speech_dbfs(
                i,
                &output_frame_levels,
                &is_active,
                &active_frames,
                LOCAL_SPEECH_RADIUS_FRAMES,
            )
        })
        .collect();
    local_after_levels.sort_by(f64::total_cmp);

    stats.applied = true;
    stats.used_original = false;
    stats.dc_removed = has_significant_dc;
    stats.gain_db = active_mean_gain_db;
    stats.min_gain_db = min_gain_db;
    stats.max_gain_db = max_gain_db;
    stats.peak_after_dbfs = linear_to_dbfs(peak_linear(&output));
    stats.active_rms_after_dbfs =
        linear_to_dbfs(rms_for_frames(&output, frame_size, &active_frames));
    stats.dynamic_spread_after_db =
        (percentile(&local_after_levels, 0.90) - percentile(&local_after_levels, 0.10)).max(0.0);

    (output, stats)
}

fn estimate_noise_floor(frame_levels: &[f64]) -> f64 {
    let mut sorted = frame_levels.to_vec();
    sorted.sort_by(f64::total_cmp);
    // Use the 5th percentile so recordings that are 80–93% speech (including
    // quiet voices mixed with loud voices) never mistake quiet speech for the
    // background noise floor.
    let p05 = percentile(&sorted, 0.05);
    if p05 > -95.0 {
        return p05;
    }
    // Digital silence (exact zeros) dominates; check if there are quiet
    // non-zero frames to estimate room tone, else treat noise floor as -90 dBFS.
    let non_silent: Vec<f64> = sorted.into_iter().filter(|&lvl| lvl > -95.0).collect();
    if non_silent.len() >= 5 {
        percentile(&non_silent, 0.05).min(-65.0)
    } else {
        -90.0
    }
}

fn local_active_speech_dbfs(
    center: usize,
    frame_levels: &[f64],
    is_active: &[bool],
    active_frames: &[usize],
    radius: usize,
) -> f64 {
    let start = center.saturating_sub(radius);
    let end = (center + radius + 1).min(frame_levels.len());
    let mut local_db = Vec::with_capacity(end - start);
    for idx in start..end {
        if is_active[idx] {
            local_db.push(frame_levels[idx]);
        }
    }
    if !local_db.is_empty() {
        // Use the 60th percentile in dB rather than linear-energy mean:
        // anchors to local speech vowels while remaining immune to a single
        // transient spike or an adjacent loud phrase bleeding across a short pause.
        local_db.sort_by(f64::total_cmp);
        return percentile(&local_db, 0.60);
    }
    // Fallback for lookahead/hangover fringe: nearest active frame.
    let nearest = match active_frames.binary_search(&center) {
        Ok(pos) => active_frames[pos],
        Err(pos) => {
            if pos == 0 {
                active_frames[0]
            } else if pos >= active_frames.len() {
                active_frames[active_frames.len() - 1]
            } else {
                let left = active_frames[pos - 1];
                let right = active_frames[pos];
                if center - left <= right - center {
                    left
                } else {
                    right
                }
            }
        }
    };
    frame_levels[nearest]
}

fn apply_subsonic_hpf_f64(samples: &[i16], sample_rate: u32) -> Vec<f64> {
    let rc = 1.0 / (2.0 * std::f64::consts::PI * SUBSONIC_HPF_HZ);
    let dt = 1.0 / sample_rate as f64;
    let alpha = rc / (rc + dt);
    let mut out = Vec::with_capacity(samples.len());
    if samples.is_empty() {
        return out;
    }
    // Initialize filter state to the first sample so a DC offset does not create
    // a startup click.
    let mut prev_x = samples[0] as f64 / 32768.0;
    let mut prev_y = 0.0_f64;
    for &sample in samples {
        let x = sample as f64 / 32768.0;
        let y = alpha * (prev_y + x - prev_x);
        prev_x = x;
        prev_y = y;
        out.push(y);
    }
    out
}

fn apply_subsonic_hpf_i16(samples: &[i16], sample_rate: u32) -> Vec<i16> {
    let threshold = db_to_linear(LIMITER_THRESHOLD_DBFS);
    let ceiling = db_to_linear(LIMITER_CEILING_DBFS);
    apply_subsonic_hpf_f64(samples, sample_rate)
        .into_iter()
        .map(|s| {
            let limited = soft_knee_limit(s, threshold, ceiling);
            (limited * 32768.0).round().clamp(-32768.0, 32767.0) as i16
        })
        .collect()
}

/// Smooth C1 soft-knee peak limiter using hyperbolic tangent above `threshold`.
fn soft_knee_limit(sample: f64, threshold: f64, ceiling: f64) -> f64 {
    let mag = sample.abs();
    if mag <= threshold {
        return sample;
    }
    let headroom = (ceiling - threshold).max(1e-6);
    let excess = (mag - threshold) / headroom;
    let shaped = threshold + headroom * excess.tanh();
    sample.signum() * shaped.min(ceiling)
}

fn unchanged_stats(samples: &[i16]) -> AudioProcessingStats {
    let peak = linear_to_dbfs(peak_linear(samples));
    AudioProcessingStats {
        applied: false,
        used_original: true,
        peak_before_dbfs: peak,
        peak_after_dbfs: peak,
        noise_floor_dbfs: -120.0,
        speech_threshold_dbfs: -120.0,
        active_rms_before_dbfs: -120.0,
        active_rms_after_dbfs: -120.0,
        ..AudioProcessingStats::default()
    }
}

fn mean_linear(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.iter().map(|&s| s as f64 / 32768.0).sum::<f64>() / samples.len() as f64
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return -120.0;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn rms_dbfs(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return -120.0;
    }
    let mean_square = samples
        .iter()
        .map(|&sample| {
            let normalized = sample as f64 / 32768.0;
            normalized * normalized
        })
        .sum::<f64>()
        / samples.len() as f64;
    linear_to_dbfs(mean_square.sqrt())
}

fn rms_dbfs_f64(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return -120.0;
    }
    let mean_square = samples.iter().map(|&s| s * s).sum::<f64>() / samples.len() as f64;
    linear_to_dbfs(mean_square.sqrt())
}

fn frame_ac_rms_dbfs(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return -120.0;
    }
    let frame_mean = mean_linear(samples);
    let mean_square = samples
        .iter()
        .map(|&sample| {
            let centered = (sample as f64 / 32768.0) - frame_mean;
            centered * centered
        })
        .sum::<f64>()
        / samples.len() as f64;
    linear_to_dbfs(mean_square.sqrt())
}

fn ac_rms_for_frames(samples: &[i16], frame_size: usize, frames: &[usize], dc_offset: f64) -> f64 {
    let mut sum_square = 0.0;
    let mut count = 0usize;
    for &frame_index in frames {
        let start = frame_index * frame_size;
        let end = (start + frame_size).min(samples.len());
        for &sample in &samples[start..end] {
            let centered = (sample as f64 / 32768.0) - dc_offset;
            sum_square += centered * centered;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        (sum_square / count as f64).sqrt()
    }
}

fn rms_for_frames(samples: &[i16], frame_size: usize, frames: &[usize]) -> f64 {
    let mut sum_square = 0.0;
    let mut count = 0usize;
    for &frame_index in frames {
        let start = frame_index * frame_size;
        let end = (start + frame_size).min(samples.len());
        for &sample in &samples[start..end] {
            let normalized = sample as f64 / 32768.0;
            sum_square += normalized * normalized;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        (sum_square / count as f64).sqrt()
    }
}

fn peak_linear(samples: &[i16]) -> f64 {
    samples
        .iter()
        .map(|&sample| (sample as f64 / 32768.0).abs())
        .fold(0.0, f64::max)
}

fn linear_to_dbfs(value: f64) -> f64 {
    if value <= 0.0 {
        -120.0
    } else {
        20.0 * value.log10()
    }
}

fn db_to_linear(value: f64) -> f64 {
    10.0_f64.powf(value / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: u32 = 16_000;

    fn sine(amplitude: f64, seconds: f64, frequency: f64) -> Vec<i16> {
        let count = (SAMPLE_RATE as f64 * seconds) as usize;
        (0..count)
            .map(|index| {
                let phase =
                    2.0 * std::f64::consts::PI * frequency * index as f64 / SAMPLE_RATE as f64;
                (phase.sin() * amplitude * 32767.0) as i16
            })
            .collect()
    }

    fn rms(samples: &[i16]) -> f64 {
        db_to_linear(rms_dbfs(samples))
    }

    #[test]
    fn silence_is_preserved() {
        let input = vec![0_i16; SAMPLE_RATE as usize];
        let (output, stats) = enhance_microphone_audio(&input, SAMPLE_RATE);
        assert_eq!(output, input);
        assert!(!stats.applied);
        assert!(stats.used_original);
        assert_eq!(stats.gain_db, 0.0);
    }

    #[test]
    fn already_well_leveled_speech_uses_original_audio_untouched() {
        let room_tone = sine(0.001, 0.8, 120.0);
        // 0.14 amplitude => ~-20.1 dBFS RMS, right inside optimal range.
        let speech = sine(0.14, 1.2, 320.0);
        let mut input = room_tone.clone();
        input.extend_from_slice(&speech);
        input.extend_from_slice(&room_tone);

        let (output, stats) = enhance_microphone_audio(&input, SAMPLE_RATE);
        assert!(!stats.applied);
        assert!(stats.used_original);
        assert_eq!(output, input);
    }

    #[test]
    fn quiet_speech_is_raised_without_raising_leading_room_tone() {
        let room_tone = sine(0.001, 1.0, 120.0);
        let speech = sine(0.025, 1.0, 440.0);
        let mut input = room_tone.clone();
        input.extend_from_slice(&speech);
        input.extend_from_slice(&room_tone);

        let (output, stats) = enhance_microphone_audio(&input, SAMPLE_RATE);
        assert!(stats.applied);
        assert!(!stats.used_original);
        assert!(stats.gain_db >= 10.0);
        assert!(stats.active_rms_after_dbfs - stats.active_rms_before_dbfs > 10.0);
        assert!(stats.peak_after_dbfs <= LIMITER_CEILING_DBFS + 0.05);

        // Stable room-tone region before the lookahead window must stay ~unity.
        let before_noise = rms(&input[SAMPLE_RATE as usize / 5..SAMPLE_RATE as usize * 3 / 5]);
        let after_noise = rms(&output[SAMPLE_RATE as usize / 5..SAMPLE_RATE as usize * 3 / 5]);
        assert!((linear_to_dbfs(after_noise) - linear_to_dbfs(before_noise)).abs() < 0.6);
    }

    #[test]
    fn quiet_and_loud_voices_with_sudden_volume_change_are_balanced_without_clipping() {
        let room_tone = sine(0.001, 1.0, 120.0);
        let quiet_voice = sine(0.02, 1.5, 280.0); // ~-37 dBFS
        let loud_voice = sine(0.65, 1.5, 340.0); // ~-6.8 dBFS (sudden jump of +30 dB)
        let mut input = room_tone.clone();
        let quiet_start = input.len();
        input.extend_from_slice(&quiet_voice);
        let quiet_end = input.len();
        let loud_start = input.len();
        input.extend_from_slice(&loud_voice);
        let loud_end = input.len();
        input.extend_from_slice(&room_tone);

        let (output, stats) = enhance_microphone_audio(&input, SAMPLE_RATE);
        assert!(stats.applied);
        assert!(
            stats.dynamic_spread_after_db < stats.dynamic_spread_before_db - 10.0,
            "dynamic spread should shrink substantially: before={:.1} after={:.1}",
            stats.dynamic_spread_before_db,
            stats.dynamic_spread_after_db
        );

        // Quiet speech interior must be meaningfully boosted.
        let quiet_before = rms_dbfs(&input[quiet_start + 4000..quiet_end - 4000]);
        let quiet_after = rms_dbfs(&output[quiet_start + 4000..quiet_end - 4000]);
        assert!(
            quiet_after - quiet_before > 10.0,
            "quiet speech should be lifted: before={:.1} after={:.1}",
            quiet_before,
            quiet_after
        );

        // Sudden loud speech must be tamed and never hard-clipped.
        let loud_before = rms_dbfs(&input[loud_start + 4000..loud_end - 4000]);
        let loud_after = rms_dbfs(&output[loud_start + 4000..loud_end - 4000]);
        assert!(
            loud_after < loud_before - 4.0,
            "loud speech should be attenuated gently: before={:.1} after={:.1}",
            loud_before,
            loud_after
        );
        assert!(output.iter().all(|&s| s < i16::MAX && s > i16::MIN));
        assert!(stats.peak_after_dbfs <= LIMITER_CEILING_DBFS + 0.05);
    }

    #[test]
    fn long_thinking_pauses_are_preserved_without_losing_quiet_speech_or_boosting_silence() {
        let long_pause = sine(0.0008, 8.0, 120.0); // 8 s thinking pause
        let short_quiet_phrase = sine(0.022, 0.7, 350.0); // only ~8% of total duration
        let mut input = long_pause.clone();
        let speech1_start = input.len();
        input.extend_from_slice(&short_quiet_phrase);
        let speech1_end = input.len();
        let pause_mid_start = input.len();
        input.extend_from_slice(&long_pause);
        let pause_mid_end = input.len();
        input.extend_from_slice(&short_quiet_phrase);

        let (output, stats) = enhance_microphone_audio(&input, SAMPLE_RATE);
        assert!(
            stats.applied,
            "quiet speech separated by 8 s pauses must still be detected and leveled"
        );
        // Total length must remain identical (no pause trimming or cutoff).
        assert_eq!(output.len(), input.len());

        // Quiet phrase lifted.
        let s1_before = rms_dbfs(&input[speech1_start + 1600..speech1_end - 1600]);
        let s1_after = rms_dbfs(&output[speech1_start + 1600..speech1_end - 1600]);
        assert!(s1_after - s1_before > 10.0);

        // Middle of 8-second thinking pause stays at original noise floor.
        let p_before = rms_dbfs(&input[pause_mid_start + 32000..pause_mid_end - 32000]);
        let p_after = rms_dbfs(&output[pause_mid_start + 32000..pause_mid_end - 32000]);
        assert!(
            (p_after - p_before).abs() < 0.6,
            "thinking pause room tone must not be boosted: before={:.1} after={:.1}",
            p_before,
            p_after
        );
    }

    #[test]
    fn steady_signal_without_noise_separation_is_not_aggressively_changed() {
        let input = sine(0.02, 2.0, 440.0);
        let (output, stats) = enhance_microphone_audio(&input, SAMPLE_RATE);
        assert_eq!(output, input);
        assert!(!stats.applied);
    }

    #[test]
    fn peak_headroom_and_soft_limiter_prevent_clipping_even_with_transient_spike() {
        let room_tone = sine(0.001, 1.0, 120.0);
        let mut speech = sine(0.025, 1.0, 440.0);
        speech[SAMPLE_RATE as usize / 2] = 30_000;
        let mut input = room_tone.clone();
        let speech_start = input.len();
        input.extend_from_slice(&speech);
        let speech_end = input.len();
        input.extend_from_slice(&room_tone);

        let (output, stats) = enhance_microphone_audio(&input, SAMPLE_RATE);
        assert!(stats.applied);
        assert!(stats.peak_after_dbfs <= LIMITER_CEILING_DBFS + 0.05);
        assert!(output
            .iter()
            .all(|&sample| sample != i16::MAX && sample != i16::MIN));
        // Quiet speech away from the spike is still lifted instead of suppressed by 1 spike.
        let before = rms_dbfs(&input[speech_start + 1600..speech_start + 5000]);
        let after = rms_dbfs(&output[speech_start + 1600..speech_start + 5000]);
        assert!(after - before > 9.0);
        let _ = speech_end;
    }

    #[test]
    fn dc_offset_is_removed_without_distorting_speech() {
        let room_tone = sine(0.001, 0.5, 120.0);
        let speech = sine(0.14, 1.0, 320.0);
        let mut input = room_tone.clone();
        input.extend_from_slice(&speech);
        input.extend_from_slice(&room_tone);
        // Inject a strong +2000 DC offset.
        for s in &mut input {
            *s = s.saturating_add(2000);
        }

        let (output, stats) = enhance_microphone_audio(&input, SAMPLE_RATE);
        assert!(stats.applied);
        assert!(stats.dc_removed);
        assert!(mean_linear(&output).abs() < 0.005);
    }
}
