import assert from "node:assert/strict";
import test from "node:test";
import { appliedSummary, entryCostUsd, formatUsd } from "../../src/lib/applied.ts";

test("summary names the features that touched a dictation", () => {
  const items = appliedSummary({
    stages: "openrouter_stt,quick_refine:openrouter,strict_literals:1,snippet:github",
    warnings: ["Github→GitHub (×2)", "backtrack_explicit_correction"],
    pipeline_runs: [{ profile_id: "codex", formatting_level: "smart" }],
  });
  assert.deepEqual(
    items.map((item) => item.label),
    ["Refinado com IA", "Vocabulário", "Snippet", "Correção falada aplicada", "Style"],
  );
  assert.equal(items[1].detail, "Github → GitHub");
});

test("safety fallbacks are flagged as caution", () => {
  const items = appliedSummary({
    stages: "offline_whisper",
    warnings: ["code_guard_rejected:index.tsx"],
    used_fallback: true,
  });
  assert.deepEqual(items.map((item) => [item.label, item.tone]), [
    ["Transcrito offline", "caution"],
    ["Refino descartado", "caution"],
  ]);
});

test("plain dictation reports nothing extra", () => {
  assert.deepEqual(appliedSummary({ stages: "openrouter_stt", pipeline_runs: [{ profile_id: "default" }] }), []);
});

test("cost sums every run and formats fractions of a cent", () => {
  assert.equal(entryCostUsd({ pipeline_runs: [{ usage: { cost: { amount_usd: 0.0002 } } }, { usage: { cost: { amount_usd: 0.0001 } } }] }).toFixed(4), "0.0003");
  assert.equal(entryCostUsd({ pipeline_runs: [{}] }), null);
  assert.equal(formatUsd(0.0003), "US$ 0,0003");
  assert.equal(formatUsd(1.5), "US$ 1,50");
});
