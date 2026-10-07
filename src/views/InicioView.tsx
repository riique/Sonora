import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ArrowRight, ChevronDown } from "lucide-react";
import { Button } from "../components/ui/Button";
import { KbdCombo, shortcutKeys } from "../components/ui/Kbd";
import { ErrorState, SkeletonRows } from "../components/ui/Surface";
import { formatDuration, shortStamp } from "../lib/format";
import { LogRow } from "../components/ui/LogRow";
import {
  getHistoryPage,
  getModeConfig,
  getOutputPolicyConfig,
  getShortcuts,
  setOutputPolicyConfig,
  type HistoryEntry,
  type ModeConfigSnapshot,
  type OutputPolicyConfig,
  type ShortcutConfig,
} from "../lib/tauri";
import { formatClock, Tally, useElapsed, type OnAirState } from "../recording/useOnAir";
import type { Navigate } from "./index";

export const MODE_LABELS: Record<string, string> = {
  "ultra-fast": "Ultrarrápido",
  "fast-accurate": "Rápido e preciso",
  precise: "Preciso",
  "ultra-precise": "Ultrapreciso",
};

/** One readable line describing the active route: model and provider. */
export function routeSummary(config: ModeConfigSnapshot | null): string {
  if (!config) return "";
  if (config.mode === "ultra-fast") {
    const whisper = config.gemini_pipelines.ultra_fast_whisper === "large-v3" ? "Whisper Large v3" : "Whisper Large v3 Turbo";
    return config.gemini_pipelines.ultra_fast_provider === "groq" ? `${whisper} direto na Groq` : `${whisper} via OpenRouter (Groq)`;
  }
  const key = config.mode === "fast-accurate" ? "fast_accurate" : config.mode === "precise" ? "precise" : "ultra_precise";
  const route = config.gemini_pipelines[key];
  if (!route) return "";
  const model = route.provider === "meta"
    ? route.custom_model || "Muse Voice Transcribe 1.0"
    : route.use_custom_model
      ? route.custom_model || "Modelo customizado"
      : route.model === "transcribe35" ? "Gemini 3.5 Transcribe" : route.model === "flash36" ? "Gemini 3.6 Flash" : "Gemini 3.5 Flash-Lite";
  const provider = route.provider === "meta" ? "Meta" : route.provider === "open-router" ? "OpenRouter" : "Google AI Studio";
  return `${model} via ${provider}`;
}

const HEADLINE: Record<OnAirState, string> = {
  off: "Pronto para ditar",
  live: "No ar",
  standby: "Processando o ditado",
};

export function InicioView({ onNavigate, onAir = "off" }: { onNavigate: Navigate; onAir?: OnAirState }) {
  const [totals, setTotals] = useState({ count: 0, words: 0 });
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [policy, setPolicy] = useState<OutputPolicyConfig | null>(null);
  const [savingPolicy, setSavingPolicy] = useState(false);
  const [error, setError] = useState("");
  const [pipeline, setPipeline] = useState<ModeConfigSnapshot | null>(null);
  const [shortcuts, setShortcutsState] = useState<ShortcutConfig>({ toggle: "Control+B", cancel: "Control+Q", command: "Control+Shift+B", hold_to_talk: false });
  const [loading, setLoading] = useState(true);

  const refresh = async () => {
    try {
      const [entries, config, output] = await Promise.all([getHistoryPage("", 0, 5), getModeConfig(), getOutputPolicyConfig()]);
      setPolicy(output);
      setError("");
      setHistory(entries.items);
      setTotals({ count: entries.total, words: entries.total_words });
      setPipeline(config);
    } catch (failure) {
      setError(String(failure));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void refresh();
    getShortcuts().then(setShortcutsState).catch(() => {});
    const saved = listen("transcription-saved", refresh);
    return () => {
      void saved.then((unlisten) => unlisten());
    };
  }, []);

  const changeOutput = async (change: Partial<Pick<OutputPolicyConfig, "destination" | "temporary_override">>) => {
    if (savingPolicy) return;
    setSavingPolicy(true);
    setError("");
    try {
      const current = await getOutputPolicyConfig();
      setPolicy(await setOutputPolicyConfig({ ...current, ...change }));
    } catch (failure) {
      setError(String(failure));
    } finally {
      setSavingPolicy(false);
    }
  };

  const recording = onAir !== "off";
  const elapsed = useElapsed(onAir);
  const enabledProfiles = policy?.profiles.filter((profile) => profile.enabled) ?? [];
  const toggleKeys = shortcutKeys(shortcuts.toggle);
  const cancelKeys = shortcutKeys(shortcuts.cancel);
  const commandKeys = shortcuts.command ? shortcutKeys(shortcuts.command) : null;
  const hold = shortcuts.hold_to_talk;

  return (
    <div>
      <section aria-labelledby="ready-title" className="pt-4">
        <div className="flex items-center gap-3">
          <Tally state={onAir} size="lg" />
          <h1 id="ready-title" className="page-title text-[32px]" aria-live="polite">{HEADLINE[onAir]}</h1>
          {onAir === "live" && <span className="ml-1 font-mono text-[32px] font-medium tabular-nums tracking-[-0.02em] text-live" role="timer" aria-label="Tempo de gravação">{formatClock(elapsed)}</span>}
        </div>
        <p className="mt-3 max-w-[56ch] text-[14px] leading-6 text-muted">
          {onAir === "live"
            ? <>Fale normalmente. {hold ? "Solte o atalho para colar" : "Pressione o atalho de novo para encerrar"} ou <KbdCombo keys={cancelKeys} /> para cancelar.</>
            : hold
              ? "Segure o atalho em qualquer aplicativo enquanto fala. Ao soltar, o texto é colado onde estiver o cursor."
              : "Pressione o atalho em qualquer aplicativo e comece a falar. O texto é colado onde estiver o cursor."}
        </p>
        <div className="mt-8 flex flex-wrap items-end gap-x-10 gap-y-4">
          <KbdCombo keys={toggleKeys} size="lg" />
          {commandKeys && onAir === "off" && (
            <p className="flex items-center gap-2.5 pb-1 text-[12.5px] text-muted">
              <KbdCombo keys={commandKeys} />
              <span>com um texto selecionado: diga o que fazer com ele</span>
            </p>
          )}
        </div>
      </section>

      {error && <div className="mt-10"><ErrorState><p>{error}</p><button type="button" className="mt-1 font-medium underline" onClick={() => void refresh()}>Tentar novamente</button></ErrorState></div>}

      <section aria-labelledby="next-take" className="mt-14 border-t border-line pt-6">
        <h2 id="next-take" className="sr-only">Próximo ditado</h2>
        <p className="max-w-[72ch] text-[14px] leading-[2.1] text-muted">
          O próximo ditado será{" "}
          <label className="inline-flex">
            <span className="sr-only">Destino do próximo ditado</span>
            <InlineSelect
              disabled={!policy || savingPolicy || recording}
              value={policy?.destination ?? "focused_field"}
              onChange={(value) => void changeOutput({ destination: value as OutputPolicyConfig["destination"] })}
              options={[["focused_field", "colado no campo em foco"], ["clipboard_only", "apenas copiado"], ["scratchpad", "salvo como nota"]]}
            />
          </label>
          {enabledProfiles.length > 0 && (
            <>
              , com estilo{" "}
              <label className="inline-flex">
                <span className="sr-only">Estilo do próximo ditado</span>
                <InlineSelect
                  disabled={!policy || savingPolicy || recording}
                  value={policy?.temporary_override ?? ""}
                  onChange={(value) => void changeOutput({ temporary_override: value || null })}
                  options={[["", "automático por aplicativo"], ...enabledProfiles.map((profile) => [profile.id, profile.name] as [string, string])]}
                />
              </label>
            </>
          )}
          , no modo <span className="font-medium text-ink">{MODE_LABELS[pipeline?.mode ?? ""] ?? "…"}</span>
          {routeSummary(pipeline) && <span> ({routeSummary(pipeline)})</span>}.{" "}
          <button
            type="button"
            onClick={() => onNavigate("ajustes", "transcricao")}
            className="font-medium text-ink underline decoration-line-strong underline-offset-4 transition-colors hover:decoration-ink"
          >
            Alterar modo
          </button>
        </p>
      </section>

      <section className="mt-14" aria-labelledby="recent-activity">
        <div className="mb-2 flex items-end justify-between gap-4">
          <div>
            <h2 id="recent-activity" className="section-title">Últimos ditados</h2>
            {totals.count > 0 && (
              <p className="meta-label mt-1 tabular-nums">
                {totals.count.toLocaleString("pt-BR")} ditados · {totals.words.toLocaleString("pt-BR")} palavras no total
              </p>
            )}
          </div>
          {totals.count > 0 && (
            <Button variant="ghost" size="sm" onClick={() => onNavigate("historico")}>
              Ver histórico <ArrowRight className="h-3.5 w-3.5" aria-hidden />
            </Button>
          )}
        </div>
        {loading ? (
          <SkeletonRows count={3} />
        ) : history.length ? (
          <ul className="hairline-list border-y border-line">
            {history.map((entry) => (
              <LogRow
                key={entry.id}
                time={shortStamp(entry.date)}
                duration={formatDuration(entry.duration_ms)}
                onOpen={() => onNavigate("historico")}
              >
                <span className={"line-clamp-1 text-[13.5px] " + (entry.is_error ? "text-live" : "text-ink")}>
                  {entry.is_error ? entry.error_message || "Falha na transcrição" : entry.text || "Transcrição sem texto"}
                </span>
              </LogRow>
            ))}
          </ul>
        ) : (
          <div className="border-y border-line py-10">
            <p className="text-[13px] font-medium text-ink">Nenhum ditado ainda</p>
            <p className="mt-1 text-[13px] text-muted">Seu primeiro ditado aparece aqui assim que você usar {toggleKeys.join(" + ")}.</p>
          </div>
        )}
      </section>
    </div>
  );
}

/** A select that reads as part of a sentence: underlined value, no box. */
function InlineSelect({ value, options, onChange, disabled }: { value: string; options: [string, string][]; onChange: (value: string) => void; disabled?: boolean }) {
  return (
    <span className="relative inline-flex items-center">
      <select
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        className="cursor-pointer appearance-none rounded-[4px] bg-transparent pr-[18px] font-medium text-ink underline decoration-line-strong decoration-1 underline-offset-4 transition-colors hover:decoration-ink disabled:cursor-not-allowed disabled:opacity-60 [&>option]:bg-raised [&>option]:text-ink"
      >
        {options.map(([optionValue, label]) => <option key={optionValue} value={optionValue}>{label}</option>)}
      </select>
      <ChevronDown className="pointer-events-none absolute right-0 h-3.5 w-3.5 text-muted" aria-hidden />
    </span>
  );
}
