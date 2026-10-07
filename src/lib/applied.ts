/**
 * Plain-language summary of what Sonora did to a dictation, derived from the
 * stage names and warnings the pipeline records. Kept pure for tests.
 */

export type AppliedTone = "neutral" | "caution";

export interface AppliedItem {
  label: string;
  detail?: string;
  tone: AppliedTone;
}

interface AppliedSource {
  stages?: string | null;
  warnings?: string[] | null;
  used_fallback?: boolean | null;
  pipeline_runs?: Array<{
    profile_id?: string | null;
    formatting_level?: string;
    usage?: { cost?: { amount_usd?: number | null } | null } | null;
  }>;
}

const VOCABULARY_HIT = /^(.+?)→(.+?) \(×(\d+)\)$/;

export function appliedSummary(entry: AppliedSource): AppliedItem[] {
  const stages = (entry.stages ?? "").split(",").map((stage) => stage.trim()).filter(Boolean);
  const warnings = entry.warnings ?? [];
  const has = (prefix: string) => stages.some((stage) => stage === prefix || stage.startsWith(`${prefix}:`));
  const items: AppliedItem[] = [];

  if (has("voice_command")) {
    items.push({
      label: stages.includes("voice_command_selection") ? "Comando de voz na seleção" : "Comando de voz",
      tone: "neutral",
    });
  }
  if (has("offline_whisper")) items.push({ label: "Transcrito offline", detail: "sem internet", tone: "caution" });
  if (has("quick_refine")) items.push({ label: "Refinado com IA", tone: "neutral" });

  const terms = warnings
    .map((warning) => VOCABULARY_HIT.exec(warning))
    .filter((match): match is RegExpExecArray => match !== null)
    .map((match) => `${match[1].trim()} → ${match[2].trim()}`);
  if (terms.length) items.push({ label: "Vocabulário", detail: terms.join(", "), tone: "neutral" });

  if (has("snippet")) items.push({ label: "Snippet", tone: "neutral" });
  if (warnings.some((warning) => warning.startsWith("backtrack_"))) items.push({ label: "Correção falada aplicada", tone: "neutral" });

  const run = entry.pipeline_runs?.[entry.pipeline_runs.length - 1];
  if (run?.profile_id && run.profile_id !== "default") items.push({ label: "Style", detail: run.profile_id, tone: "neutral" });
  if (run?.formatting_level === "literal") items.push({ label: "Formatação literal", tone: "neutral" });
  if (run?.formatting_level === "aggressive") items.push({ label: "Formatação reorganizada", tone: "neutral" });

  if (warnings.some((warning) => warning.startsWith("code_guard_rejected"))) {
    items.push({ label: "Refino descartado", detail: "alteraria código ou nomes de arquivo", tone: "caution" });
  }
  if (warnings.includes("quick_refine_failed")) items.push({ label: "IA indisponível", detail: "texto do Whisper mantido", tone: "caution" });
  if (entry.used_fallback && !has("offline_whisper")) items.push({ label: "Fallback de provedor", tone: "caution" });

  return items;
}

export function entryCostUsd(entry: AppliedSource): number | null {
  const amounts = (entry.pipeline_runs ?? [])
    .map((run) => run.usage?.cost?.amount_usd)
    .filter((amount): amount is number => typeof amount === "number");
  return amounts.length ? amounts.reduce((sum, amount) => sum + amount, 0) : null;
}

/** US$ with enough precision for fractions of a cent. */
export function formatUsd(amount: number): string {
  const digits = amount === 0 ? 2 : amount < 0.01 ? 4 : 2;
  return `US$ ${amount.toLocaleString("pt-BR", { minimumFractionDigits: digits, maximumFractionDigits: digits })}`;
}
