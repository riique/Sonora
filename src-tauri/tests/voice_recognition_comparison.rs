//! Comparative accuracy and acoustic preservation tests for Sonora's voice
//! recognition pipeline:
//! - Original vs processed audio fidelity
//! - Quiet speech ("falas baixas")
//! - Sudden volume changes inside the same recording
//! - Multi-second thinking pauses (without cutoff or silence hallucinations)
//! - Brazilian Portuguese mixed with English (code-switching) & AssemblyAI params

use sonora_lib::{
    assemblyai, audio_processing, openrouter,
    pipeline_contract::{TranscriptionModel, TranscriptionProvider, TranscriptionRoute},
    speech_presence, transformations, vocabulary,
};

const SAMPLE_RATE: u32 = 16_000;

fn load_wav_pcm(bytes: &[u8]) -> Vec<i16> {
    assert_eq!(&bytes[..4], b"RIFF");
    let mut cursor = 12;
    while cursor + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let chunk = &bytes[cursor + 8..cursor + 8 + size];
        if &bytes[cursor..cursor + 4] == b"data" {
            return chunk
                .chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                .collect();
        }
        cursor += 8 + size + (size % 2);
    }
    panic!("WAV fixture missing data chunk");
}

fn rms_dbfs(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return -120.0;
    }
    let ms = samples
        .iter()
        .map(|&s| {
            let v = s as f64 / 32768.0;
            v * v
        })
        .sum::<f64>()
        / samples.len() as f64;
    if ms <= 0.0 {
        -120.0
    } else {
        10.0 * ms.log10()
    }
}

fn scale_to_rms_dbfs(samples: &[i16], target_dbfs: f64) -> Vec<i16> {
    let current = rms_dbfs(samples);
    let gain = 10.0_f64.powf((target_dbfs - current) / 20.0);
    samples
        .iter()
        .map(|&s| (s as f64 * gain).round().clamp(-32768.0, 32767.0) as i16)
        .collect()
}

/// Low-level deterministic room tone (synthetic HVAC/mic floor ~ -62 dBFS).
fn room_tone(seconds: f64, amplitude: f64) -> Vec<i16> {
    let count = (SAMPLE_RATE as f64 * seconds) as usize;
    (0..count)
        .map(|i| {
            let t = i as f64 / SAMPLE_RATE as f64;
            let hum = (std::f64::consts::TAU * 120.0 * t).sin() * 0.5
                + (std::f64::consts::TAU * 240.0 * t).sin() * 0.3
                + (std::f64::consts::TAU * 370.0 * t).sin() * 0.2;
            (hum * amplitude * 32767.0).round() as i16
        })
        .collect()
}

/// Pearson correlation between two equal-length PCM slices (checks waveform
/// fidelity / absence of phoneme distortion).
fn pearson_correlation(a: &[i16], b: &[i16]) -> f64 {
    assert_eq!(a.len(), b.len());
    let n = a.len() as f64;
    let mean_a = a.iter().map(|&x| x as f64).sum::<f64>() / n;
    let mean_b = b.iter().map(|&x| x as f64).sum::<f64>() / n;
    let mut num = 0.0;
    let mut da = 0.0;
    let mut db = 0.0;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let vx = x as f64 - mean_a;
        let vy = y as f64 - mean_b;
        num += vx * vy;
        da += vx * vx;
        db += vy * vy;
    }
    if da == 0.0 || db == 0.0 {
        return 0.0;
    }
    num / (da.sqrt() * db.sqrt())
}

#[test]
fn comparison_original_vs_processed_keeps_healthy_audio_original_and_lifts_quiet_speech() {
    let phrase = load_wav_pcm(include_bytes!(
        "../../tests/fixtures/speech/frase-curta.wav"
    ));

    // Case A: Healthy well-leveled recording (-20 dBFS speech) -> MUST stay bit-identical original.
    let healthy_speech = scale_to_rms_dbfs(&phrase, -20.0);
    let mut healthy_recording = room_tone(0.6, 0.0008);
    healthy_recording.extend_from_slice(&healthy_speech);
    healthy_recording.extend_from_slice(&room_tone(0.6, 0.0008));

    let (healthy_out, healthy_stats) =
        audio_processing::enhance_microphone_audio(&healthy_recording, SAMPLE_RATE);
    assert!(
        healthy_stats.used_original && !healthy_stats.applied,
        "healthy speech must pass original audio untouched"
    );
    assert_eq!(healthy_out, healthy_recording);

    // Case B: Quiet recording (-36 dBFS speech) -> processed lifts speech cleanly
    // while preserving waveform fidelity (no phoneme distortion) and room tone.
    let quiet_speech = scale_to_rms_dbfs(&phrase, -36.0);
    let lead = room_tone(0.8, 0.0008);
    let mut quiet_recording = lead.clone();
    let speech_start = quiet_recording.len();
    quiet_recording.extend_from_slice(&quiet_speech);
    let speech_end = quiet_recording.len();
    quiet_recording.extend_from_slice(&lead);

    assert!(!speech_presence::is_clearly_silent(
        &quiet_recording,
        SAMPLE_RATE
    ));
    let (quiet_out, quiet_stats) =
        audio_processing::enhance_microphone_audio(&quiet_recording, SAMPLE_RATE);
    assert!(quiet_stats.applied && !quiet_stats.used_original);

    let orig_speech_db = rms_dbfs(&quiet_recording[speech_start..speech_end]);
    let proc_speech_db = rms_dbfs(&quiet_out[speech_start..speech_end]);
    assert!(
        proc_speech_db - orig_speech_db > 10.0,
        "quiet speech should gain >10 dB (orig={orig_speech_db:.1}, proc={proc_speech_db:.1})"
    );
    let corr = pearson_correlation(
        &quiet_recording[speech_start..speech_end],
        &quiet_out[speech_start..speech_end],
    );
    assert!(
        corr > 0.96,
        "processed quiet speech must preserve phoneme waveform fidelity (corr={corr:.4})"
    );
}

#[test]
fn comparison_quiet_speech_fixtures_survive_admission_and_leveling() {
    let fixtures: &[(&str, &[u8])] = &[
        ("oi", include_bytes!("../../tests/fixtures/speech/oi.wav")),
        ("sim", include_bytes!("../../tests/fixtures/speech/sim.wav")),
        (
            "frase-curta",
            include_bytes!("../../tests/fixtures/speech/frase-curta.wav"),
        ),
    ];

    for (name, wav) in fixtures {
        let pcm = load_wav_pcm(wav);
        let quiet = scale_to_rms_dbfs(&pcm, -38.0);
        let mut recording = room_tone(0.7, 0.0006);
        let start = recording.len();
        recording.extend_from_slice(&quiet);
        let end = recording.len();
        recording.extend_from_slice(&room_tone(0.7, 0.0006));

        assert!(
            !speech_presence::is_clearly_silent(&recording, SAMPLE_RATE),
            "quiet fixture '{name}' must never be rejected as silence"
        );
        let (processed, stats) =
            audio_processing::enhance_microphone_audio(&recording, SAMPLE_RATE);
        assert!(stats.applied, "quiet fixture '{name}' should be leveled");
        assert!(
            rms_dbfs(&processed[start..end]) > rms_dbfs(&recording[start..end]) + 9.0,
            "quiet fixture '{name}' should be lifted into clear speech range"
        );
        assert!(processed.iter().all(|&s| s != i16::MAX && s != i16::MIN));
    }
}

#[test]
fn comparison_sudden_volume_changes_in_same_recording_preserve_both_quiet_and_loud() {
    let phrase = load_wav_pcm(include_bytes!(
        "../../tests/fixtures/speech/frase-curta.wav"
    ));
    let quiet_part = scale_to_rms_dbfs(&phrase, -37.0);
    let loud_part = scale_to_rms_dbfs(&phrase, -8.0);
    let short_gap = room_tone(0.4, 0.0008);

    let mut recording = room_tone(0.8, 0.0008);
    let quiet1_start = recording.len();
    recording.extend_from_slice(&quiet_part);
    let quiet1_end = recording.len();
    recording.extend_from_slice(&short_gap);
    let loud_start = recording.len();
    recording.extend_from_slice(&loud_part);
    let loud_end = recording.len();
    recording.extend_from_slice(&short_gap);
    let quiet2_start = recording.len();
    recording.extend_from_slice(&quiet_part);
    let quiet2_end = recording.len();
    recording.extend_from_slice(&room_tone(0.8, 0.0008));

    let (processed, stats) = audio_processing::enhance_microphone_audio(&recording, SAMPLE_RATE);
    assert!(stats.applied);

    let orig_gap_db =
        rms_dbfs(&recording[loud_start..loud_end]) - rms_dbfs(&recording[quiet1_start..quiet1_end]);
    let proc_gap_db =
        rms_dbfs(&processed[loud_start..loud_end]) - rms_dbfs(&processed[quiet1_start..quiet1_end]);

    assert!(
        orig_gap_db > 25.0,
        "test fixture should represent a >25 dB sudden jump (got {orig_gap_db:.1} dB)"
    );
    assert!(
        proc_gap_db < orig_gap_db - 12.0,
        "dynamic leveling must significantly compress quiet-vs-loud gap (orig={orig_gap_db:.1} dB, proc={proc_gap_db:.1} dB)"
    );

    let q1_before = rms_dbfs(&recording[quiet1_start..quiet1_end]);
    let q1_after = rms_dbfs(&processed[quiet1_start..quiet1_end]);
    let l_before = rms_dbfs(&recording[loud_start..loud_end]);
    let l_after = rms_dbfs(&processed[loud_start..loud_end]);
    let q2_before = rms_dbfs(&recording[quiet2_start..quiet2_end]);
    let q2_after = rms_dbfs(&processed[quiet2_start..quiet2_end]);
    assert!(
        q2_after - q2_before > 8.0,
        "quiet speech right after a loud burst must recover cleanly (q1={q1_before:.1}->{q1_after:.1}, loud={l_before:.1}->{l_after:.1}, q2={q2_before:.1}->{q2_after:.1}, stats={stats:?}, phrase_len={})",
        phrase.len()
    );

    // Zero hard clipping anywhere in the loud section.
    assert!(processed.iter().all(|&s| s != i16::MAX && s != i16::MIN));
    assert!(stats.peak_after_dbfs <= -0.95);
}

#[test]
fn comparison_long_thinking_pauses_are_preserved_without_cutting_words_or_amplifying_silence() {
    let oi = scale_to_rms_dbfs(
        &load_wav_pcm(include_bytes!("../../tests/fixtures/speech/oi.wav")),
        -35.0,
    );
    let phrase = scale_to_rms_dbfs(
        &load_wav_pcm(include_bytes!(
            "../../tests/fixtures/speech/frase-curta.wav"
        )),
        -35.0,
    );
    let sim = scale_to_rms_dbfs(
        &load_wav_pcm(include_bytes!("../../tests/fixtures/speech/sim.wav")),
        -35.0,
    );

    let pause_6s = room_tone(6.0, 0.0007);
    let pause_8s = room_tone(8.0, 0.0007);

    let mut recording = room_tone(1.0, 0.0007);
    let w1_start = recording.len();
    recording.extend_from_slice(&oi);
    let w1_end = recording.len();
    let p1_start = recording.len();
    recording.extend_from_slice(&pause_6s);
    let p1_end = recording.len();
    let w2_start = recording.len();
    recording.extend_from_slice(&phrase);
    let w2_end = recording.len();
    let p2_start = recording.len();
    recording.extend_from_slice(&pause_8s);
    let p2_end = recording.len();
    let w3_start = recording.len();
    recording.extend_from_slice(&sim);
    let w3_end = recording.len();
    recording.extend_from_slice(&room_tone(1.0, 0.0007));

    assert!(
        !speech_presence::is_clearly_silent(&recording, SAMPLE_RATE),
        "long pauses between words must never cause local silence rejection"
    );

    let (processed, stats) = audio_processing::enhance_microphone_audio(&recording, SAMPLE_RATE);
    assert!(stats.applied);
    assert_eq!(
        processed.len(),
        recording.len(),
        "recording duration must be preserved verbatim (never cut pauses or words)"
    );

    // Every word before, between, and after the 6 s / 8 s thinking pauses is lifted
    // cleanly into the clear speech band (short single-word WAVs like "oi" include
    // internal silence padding, so their active vowel reaches ~-20 dBFS at +7.5 dB whole-slice lift).
    for (label, start, end) in [
        ("oi", w1_start, w1_end),
        ("frase-curta", w2_start, w2_end),
        ("sim", w3_start, w3_end),
    ] {
        let before = rms_dbfs(&recording[start..end]);
        let after = rms_dbfs(&processed[start..end]);
        assert!(
            after - before > 7.5,
            "segment '{label}' surrounded by long pauses must be leveled (before={before:.1}, after={after:.1})"
        );
    }

    // Both multi-second pauses remain at original room tone (no noise amplification -> no hallucinations).
    for (label, start, end) in [
        ("pause-6s", p1_start, p1_end),
        ("pause-8s", p2_start, p2_end),
    ] {
        let margin = SAMPLE_RATE as usize; // 1 s away from speech margins
        let before = rms_dbfs(&recording[start + margin..end - margin]);
        let after = rms_dbfs(&processed[start + margin..end - margin]);
        assert!(
            (after - before).abs() < 0.6,
            "{label} room tone must not be amplified (before={before:.1}, after={after:.1})"
        );
    }
}

#[test]
fn comparison_portuguese_mixed_with_english_and_assemblyai_comparison_routes() {
    // Verify direct AssemblyAI-STT-3.7 vs OpenRouter Universal-3.6 Pro route coexistence.
    let direct_37 = TranscriptionRoute {
        provider: TranscriptionProvider::AssemblyAi,
        model: TranscriptionModel::AssemblyAiStt37,
        use_custom_model: false,
        custom_model: String::new(),
    }
    .validate()
    .unwrap();
    let openrouter_36 = TranscriptionRoute {
        provider: TranscriptionProvider::OpenRouter,
        model: TranscriptionModel::AssemblyAiUniversal36Pro,
        use_custom_model: false,
        custom_model: String::new(),
    }
    .validate()
    .unwrap();
    assert_eq!(direct_37.model_id(), "assemblyai-stt-3-7");
    assert_eq!(openrouter_36.model_id(), "assemblyai/universal-3-6-pro");
    assert!(openrouter::is_assemblyai_model(openrouter_36.model_id()));

    // Verify custom vocabulary feeds AssemblyAI cleanly (no mishearing aliases, strict first).
    let vocab = vec![
        vocabulary::VocabularyTerm {
            canonical: "AssemblyAI".into(),
            aliases: vec!["assembri ei ai".into(), "assembly ai".into()],
            category: vocabulary::VocabularyCategory::Provider,
            strict: true,
            enabled: true,
        },
        vocabulary::VocabularyTerm {
            canonical: "pull request".into(),
            aliases: vec!["pula request".into()],
            category: vocabulary::VocabularyCategory::Command,
            strict: true,
            enabled: true,
        },
        vocabulary::VocabularyTerm {
            canonical: "speech_models".into(),
            aliases: vec!["speech models".into()],
            category: vocabulary::VocabularyCategory::Identifier,
            strict: true,
            enabled: true,
        },
        vocabulary::VocabularyTerm {
            canonical: "Sonora".into(),
            aliases: vec![],
            category: vocabulary::VocabularyCategory::Application,
            strict: false,
            enabled: true,
        },
    ];

    let keyterms = assemblyai::build_keyterms(&vocab, assemblyai::MAX_DIRECT_KEYTERMS);
    assert_eq!(
        keyterms,
        vec!["AssemblyAI", "pull request", "speech_models", "Sonora"]
    );
    assert!(!keyterms.iter().any(|t| t.contains("assembri")));

    // Verify mixed pt-BR + English transcript passes strict vocabulary + formatting + code guard cleanly.
    let mixed_raw =
        "Abre o pula request do Sonora usando assembly ai com speech models para fazer deploy";
    let (after_vocab, hits) = vocabulary::apply_strict_literals(mixed_raw, &vocab);
    assert_eq!(
        after_vocab,
        "Abre o pull request do Sonora usando AssemblyAI com speech_models para fazer deploy"
    );
    assert_eq!(hits.len(), 3);

    let guarded = transformations::enforce_protected_spans(&after_vocab, &after_vocab);
    assert_eq!(guarded.text, after_vocab);
    assert!(guarded.warnings.is_empty());
}
