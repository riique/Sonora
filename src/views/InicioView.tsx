import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ArrowRight } from "lucide-react";
import { Button } from "../components/ui/Button";
import { KbdCombo, shortcutKeys } from "../components/ui/Kbd";
import { ErrorState, SkeletonRows } from "../components/ui/Surface";
import { formatDuration, shortStamp } from "../lib/format";
import { LogRow } from "../components/ui/LogRow";
import {
  getHistoryPage,
  getShortcuts,
  getTranscriptionRoute,
  type HistoryEntry,
  type ShortcutConfig,
  type TranscriptionRoute,
} from "../lib/tauri";
import { formatClock, Tally, useElapsed, type OnAirState } from "../recording/useOnAir";
import type { Navigate } from "./index";

const MODEL_LABELS: Record<TranscriptionRoute["model"], string> = {
  "assemblyai-stt-3-7": "AssemblyAI-STT-3.7",
  "assemblyai-universal-3-6-pro": "AssemblyAI Universal-3.6 Pro",
  "openai-whisper-large-v3": "OpenAI Whisper Large v3",
  "openai-whisper-large-v3-turbo": "OpenAI Whisper Large v3 Turbo",
  "openai-gpt-4o-transcribe": "OpenAI GPT-4o Transcribe",
  "openai-gpt-4o-mini-transcribe": "OpenAI GPT-4o Mini Transcribe",
};

/** One readable line describing the active route: model and provider. */
export function routeSummary(route: TranscriptionRoute | null): string {
  if (!route) return "";
  const provider = route.provider === "open-router" ? "OpenRouter" : "AssemblyAI";
  const modelLabel = route.provider === "open-router" && route.use_custom_model && route.custom_model?.trim()
    ? route.custom_model.trim()
    : (MODEL_LABELS[route.model] ?? route.model);
  return `${modelLabel} via ${provider}`;
}

const HEADLINE: Record<OnAirState, string> = {
  off: "Pronto para ditar",
  live: "No ar",
  standby: "Processando o ditado",
};

export function InicioView({ onNavigate, onAir = "off" }: { onNavigate: Navigate; onAir?: OnAirState }) {
  const [totals, setTotals] = useState({ count: 0, words: 0 });
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [error, setError] = useState("");
  const [route, setRoute] = useState<TranscriptionRoute | null>(null);
  const [shortcuts, setShortcutsState] = useState<ShortcutConfig>({ toggle: "Control+B", cancel: "Control+Q", command: "Control+Shift+B", hold_to_talk: false });
  const [loading, setLoading] = useState(true);

  const refresh = async () => {
    try {
      const [entries, config] = await Promise.all([getHistoryPage("", 0, 5), getTranscriptionRoute()]);
      setError("");
      setHistory(entries.items);
      setTotals({ count: entries.total, words: entries.total_words });
      setRoute(config);
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

  const elapsed = useElapsed(onAir);
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
          Os ditados usam <span className="font-medium text-ink">{routeSummary(route) || "…"}</span>, com Whisper de reserva.{" "}
          <button
            type="button"
            onClick={() => onNavigate("ajustes", "transcricao")}
            className="font-medium text-ink underline decoration-line-strong underline-offset-4 transition-colors hover:decoration-ink"
          >
            Alterar modelo
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
