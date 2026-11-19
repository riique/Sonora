import assert from "node:assert/strict";
import test from "node:test";
import { evaluate } from "../../scripts/benchmark.mjs";

const COMPARATIVE_SCENARIOS = [
  {
    scenario: "original-vs-processed-quiet-speech",
    reference: "Precisamos revisar o pipeline de transcrição em português brasileiro antes do release.",
    terms: ["pipeline", "release"],
    original_hypothesis: "Precisamos avisar o papel de transcrição em português brasileiro antes do relise.",
    processed_hypothesis: "Precisamos revisar o pipeline de transcrição em português brasileiro antes do release.",
  },
  {
    scenario: "sudden-volume-changes",
    reference: "Primeiro eu falo bem baixo e depois aumento o volume de repente sem perder nenhuma palavra.",
    terms: [],
    original_hypothesis: "Primeiro eu falo bem e depois aumento o volume de repente sem perder palavra.",
    processed_hypothesis: "Primeiro eu falo bem baixo e depois aumento o volume de repente sem perder nenhuma palavra.",
  },
  {
    scenario: "long-thinking-pauses",
    reference: "Deixa eu pensar um pouco antes de continuar a explicação do problema técnico.",
    terms: [],
    original_hypothesis: "Deixa eu pensar um pouco. Legenda por Sônia Ruberti.",
    processed_hypothesis: "Deixa eu pensar um pouco antes de continuar a explicação do problema técnico.",
  },
  {
    scenario: "pt-br-en-code-switching",
    reference: "Suba o audio para /v2/upload e envie speech_models com assemblyai-stt-3-7 no payload JSON.",
    terms: ["/v2/upload", "speech_models", "assemblyai-stt-3-7", "JSON"],
    original_hypothesis: "Suba o áudio para barra v2 upload e envie speech models com assembly stt 37 no payload json.",
    processed_hypothesis: "Suba o audio para /v2/upload e envie speech_models com assemblyai-stt-3-7 no payload JSON.",
  },
];

test("comparative benchmark measures WER, CER and literal terms between original and processed pipelines", () => {
  const originalRecords = COMPARATIVE_SCENARIOS.map((item) => ({
    reference: item.reference,
    hypothesis: item.original_hypothesis,
    terms: item.terms,
    latency_ms: 1450,
    cost_usd: 0.0012,
    audio_seconds: 8,
  }));
  const processedRecords = COMPARATIVE_SCENARIOS.map((item) => ({
    reference: item.reference,
    hypothesis: item.processed_hypothesis,
    terms: item.terms,
    latency_ms: 1320,
    cost_usd: 0.0011,
    audio_seconds: 8,
  }));

  const originalMetrics = evaluate(originalRecords);
  const processedMetrics = evaluate(processedRecords);

  assert.equal(originalMetrics.segments, 4);
  assert.equal(processedMetrics.segments, 4);
  assert.ok(processedMetrics.wer < originalMetrics.wer, "processed pipeline should lower WER");
  assert.ok(processedMetrics.cer < originalMetrics.cer, "processed pipeline should lower CER");
  assert.equal(processedMetrics.wer, 0);
  assert.equal(processedMetrics.literal_term_rate, 1);
  assert.ok(originalMetrics.literal_term_rate < 0.5);
});

test("comparative benchmark supports comparing AssemblyAI-STT-3.7 direct against Universal-3.6 Pro", () => {
  const stt37 = evaluate([
    {
      reference: "Integre o assemblyai-stt-3-7 no Sonora preservando code-switching e vocabulário customizado.",
      hypothesis: "Integre o assemblyai-stt-3-7 no Sonora preservando code-switching e vocabulário customizado.",
      terms: ["assemblyai-stt-3-7", "Sonora", "code-switching"],
      latency_ms: 1180,
      audio_seconds: 6,
    },
  ]);
  const universal36 = evaluate([
    {
      reference: "Integre o assemblyai-stt-3-7 no Sonora preservando code-switching e vocabulário customizado.",
      hypothesis: "Integre o assembly stt 3.7 no Sonora preservando code-switching e vocabulário customizado.",
      terms: ["assemblyai-stt-3-7", "Sonora", "code-switching"],
      latency_ms: 1290,
      audio_seconds: 6,
    },
  ]);

  assert.equal(stt37.literal_term_rate, 1);
  assert.ok(stt37.wer <= universal36.wer);
  assert.ok(universal36.literal_term_rate < 1);
});
