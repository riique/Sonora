import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowRight,
  FileText,
  Settings2,
  Zap,
} from "lucide-react";
import { Button } from "../components/ui/Button";
import { KbdCombo } from "../components/ui/Kbd";
import { PageHeader, SkeletonRows } from "../components/ui/Surface";
import {
  getHistoryPage,
  getModeConfig,
  getOutputPolicyConfig,
  setOutputPolicyConfig,
  type OutputPolicyConfig,
  getRecordingStatus,
  getShortcuts,
  onRecordingEvent,
  type HistoryEntry,
  type ModeConfigSnapshot,
  type ShortcutConfig,
} from "../lib/tauri";
import { shouldApplyRecordingStatus } from "../recording/status";
import type { ViewKey } from "./index";

const MODE_LABELS: Record<string, string> = {
  "ultra-fast": "Ultrarrápido",
  "fast-accurate": "Rápido e preciso",
  precise: "Preciso",
  "ultra-precise": "Ultrapreciso",
};

function formatEntryDuration(ms?: number): string {
  if (!ms) return "—";
  const seconds = Math.max(1, Math.round(ms / 1000));
  const minutes = Math.floor(seconds / 60);
  return minutes ? `${minutes}:${String(seconds % 60).padStart(2, "0")}` : `${seconds}s`;
}

function routeDetails(config: ModeConfigSnapshot | null) {
  if (!config) return { engine: "Carregando…", model: "—", provider: "—" };
  if (config.mode === "ultra-fast") {
    const direct = config.gemini_pipelines.ultra_fast_provider === "groq";
    return {
      engine: "Whisper · baixa latência",
      model: config.gemini_pipelines.ultra_fast_whisper === "large-v3" ? "Whisper Large v3" : "Whisper Large v3 Turbo",
      provider: direct ? "Groq direto" : "OpenRouter · Groq",
    };
  }
  const key =
    config.mode === "fast-accurate"
      ? "fast_accurate"
      : config.mode === "precise"
        ? "precise"
        : "ultra_precise";
  const route = config.gemini_pipelines[key];
  return {
    engine: config.mode === "fast-accurate" ? (route.provider === "meta" ? "Muse com áudio WAV" : "Modelo de áudio") : config.mode === "precise" ? "Whisper + Gemini" : "Whisper + validador + Gemini",
    model: route.provider === "meta" ? (route.custom_model || "Muse Voice Transcribe 1.0") : route.use_custom_model
      ? route.custom_model || "Modelo customizado"
      : route.model === "transcribe35"
        ? "Gemini 3.5 Transcribe"
        : route.model === "flash36"
          ? "Gemini 3.6 Flash"
          : "Gemini 3.5 Flash-Lite",
    provider: route.provider === "meta" ? "Meta Model API" : route.provider === "open-router" ? "OpenRouter" : "Google AI Studio",
  };
}

export function InicioView({ onNavigate }: { onNavigate: (view: ViewKey) => void }) {
  const [totals, setTotals] = useState({ count: 0, words: 0 });
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [policy, setPolicy] = useState<OutputPolicyConfig | null>(null);
  const [savingPolicy, setSavingPolicy] = useState(false);
  const [error, setError] = useState("");
  const [pipeline, setPipeline] = useState<ModeConfigSnapshot | null>(null);
  const [shortcuts, setShortcutsState] = useState<ShortcutConfig>({ toggle: "Control+B", cancel: "Control+Q" });
  const [recording, setRecording] = useState(false);
  const [loading, setLoading] = useState(true);
  const latestRecordingRevision = useRef(-1);

  const refresh = async () => {
    try {
      const [entries, config, output] = await Promise.all([getHistoryPage("", 0, 5), getModeConfig(), getOutputPolicyConfig()]);
      setPolicy(output);
      setError("");
      setHistory(entries.items);
      setTotals({ count: entries.total, words: entries.total_words });
      setPipeline(config);
    } catch (error) {
      setError(String(error));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    let mounted = true;
    let unlistenRecording: (() => void) | undefined;
    const applyRecordingStatus = (status: Awaited<ReturnType<typeof getRecordingStatus>>) => {
      if (!mounted || !shouldApplyRecordingStatus(latestRecordingRevision.current, status)) return;
      latestRecordingRevision.current = status.revision;
      setRecording(status.recording);
    };

    void refresh();
    getShortcuts().then(setShortcutsState).catch(() => {});
    const saved = listen("transcription-saved", refresh);
    void onRecordingEvent((_type, status) => applyRecordingStatus(status))
      .then(async (unlisten) => {
        if (!mounted) {
          unlisten();
          return;
        }
        unlistenRecording = unlisten;
        applyRecordingStatus(await getRecordingStatus());
      })
      .catch((error) => console.error("Failed to sync recording status:", error));
    return () => {
      mounted = false;
      saved.then((unlisten) => unlisten());
      unlistenRecording?.();
    };
  }, []);

  const changeOutput = async (change: Partial<Pick<OutputPolicyConfig, "destination" | "temporary_override">>) => {
    if (savingPolicy) return; setSavingPolicy(true); setError("");
    try { const current = await getOutputPolicyConfig(); setPolicy(await setOutputPolicyConfig({ ...current, ...change })); }
    catch (failure) { setError(String(failure)); }
    finally { setSavingPolicy(false); }
  };

  const details = routeDetails(pipeline);
  const recent = history.slice(0, 3);
  const toggleKeys = shortcuts.toggle
    .split("+")
    .map((key) => (key === "Control" || key === "CommandOrControl" ? "Ctrl" : key));

  return (
    <div>
      <PageHeader
        title="Sonora"
        description="Comece a falar em qualquer aplicativo com o atalho global."
        action={<KbdCombo keys={toggleKeys} />}
      />

      {error && <div role="alert" className="mb-5 flex flex-wrap items-center gap-3 text-sm"><p className="break-words">{error}</p><Button size="sm" onClick={() => void refresh()}>Tentar novamente</Button></div>}
      <div className="mb-6 flex flex-wrap items-end gap-4">
        <label className="flex min-w-[180px] flex-1 flex-col gap-1 text-sm">Destino do próximo ditado
          <select className="h-9 min-w-0 rounded-[9px] border border-line bg-white px-3 text-[13px] text-ink focus:border-[#9b9c95]" disabled={!policy || savingPolicy || recording} value={policy?.destination ?? "focused_field"} onChange={(e) => void changeOutput({ destination: e.target.value as OutputPolicyConfig["destination"] })}>
            <option value="focused_field">Campo em foco</option><option value="clipboard_only">Área de transferência</option><option value="scratchpad">Scratchpad</option>
          </select>
        </label>
        <label className="flex min-w-[180px] flex-1 flex-col gap-1 text-sm">Style temporário
          <select className="h-9 min-w-0 rounded-[9px] border border-line bg-white px-3 text-[13px] text-ink focus:border-[#9b9c95]" disabled={!policy || savingPolicy || recording} value={policy?.temporary_override ?? ""} onChange={(e) => void changeOutput({ temporary_override: e.target.value || null })}>
            <option value="">Automático por aplicativo</option>{policy?.profiles.filter((profile) => profile.enabled).map((profile) => <option key={profile.id} value={profile.id}>{profile.name}</option>)}
          </select>
        </label>
        <Button onClick={() => onNavigate("recuperacao")}>Verificar configuração</Button>
      </div>

      {recording && (
        <div className="mb-6 flex items-center gap-2 rounded-[10px] bg-[#fff1ef] px-4 py-3 text-[13px] font-medium text-[#9f2720]" role="status">
          <span className="h-2 w-2 rounded-full bg-[#c2392f] animate-quiet-pulse" />
          Gravação em andamento no gadget
        </div>
      )}

      <section className="surface px-6 py-5" aria-labelledby="active-pipeline">
        <div className="flex items-start justify-between gap-5">
          <div className="min-w-0">
            <h2 id="active-pipeline" className="meta-label">Pipeline ativa</h2>
            <div className="mt-2 flex items-center gap-3">
              <span className="flex h-8 w-8 items-center justify-center rounded-[9px] bg-[#efefeb] text-ink">
                <Zap className="h-4 w-4" strokeWidth={1.8} aria-hidden />
              </span>
              <div>
                <p className="text-[17px] font-semibold tracking-[-0.015em] text-ink">
                  {MODE_LABELS[pipeline?.mode ?? ""] ?? "Carregando…"}
                </p>
                <p className="mt-0.5 text-[12px] text-muted">{details.engine}</p>
              </div>
            </div>
          </div>
          <Button variant="secondary" size="sm" onClick={() => onNavigate("configuracoes")}>
            <Settings2 className="h-3.5 w-3.5" aria-hidden />
            Configurar
          </Button>
        </div>
        <dl className="mt-5 grid grid-cols-3 divide-x divide-line border-t border-line pt-4 max-[980px]:grid-cols-1 max-[980px]:divide-x-0 max-[980px]:divide-y">
          <PipelineFact label="Modelo" value={details.model} />
          <PipelineFact label="Provedor" value={details.provider} />
          <PipelineFact label="FileTagging" value={pipeline ? (pipeline.file_tagging_enabled ? "Ativo" : "Desativado") : "Carregando…"} />
        </dl>
      </section>

      <section className="mt-8" aria-labelledby="recent-activity">
        <div className="mb-3 flex items-center justify-between">
          <h2 id="recent-activity" className="section-title">Atividade recente <span className="text-sm font-normal text-muted">· {totals.count.toLocaleString("pt-BR")} ditados</span></h2>
          <Button variant="ghost" size="sm" onClick={() => onNavigate("historico")}>
            Ver histórico <ArrowRight className="h-3.5 w-3.5" aria-hidden />
          </Button>
        </div>
        <div className="surface">
          {loading ? (
            <SkeletonRows count={3} />
          ) : recent.length ? (
            <div className="divider-list">
              {recent.map((entry) => (
                <button
                  key={entry.id}
                  type="button"
                  onClick={() => onNavigate("historico")}
                  className="flex w-full items-center gap-4 px-5 py-4 text-left transition-colors hover:bg-[#fafaf8] first:rounded-t-[14px] last:rounded-b-[14px]"
                >
                  <FileText className="h-4 w-4 shrink-0 text-[#7a7b74]" strokeWidth={1.7} aria-hidden />
                  <div className="min-w-0 flex-1">
                    <p className={`truncate text-[13px] font-medium ${entry.is_error ? "text-[#a72a21]" : "text-ink"}`}>
                      {entry.is_error ? entry.error_message || "Falha na transcrição" : entry.text || "Transcrição sem texto"}
                    </p>
                    <p className="mt-1 truncate text-[11px] text-muted">{entry.date}</p>
                  </div>
                  <span className="shrink-0 text-[11px] tabular-nums text-muted">{formatEntryDuration(entry.duration_ms)}</span>
                </button>
              ))}
            </div>
          ) : (
            <div className="px-5 py-10 text-center">
              <p className="text-[13px] font-medium text-ink">Nenhuma atividade ainda</p>
              <p className="mt-1 text-[12px] text-muted">Use {toggleKeys.join(" + ")} para criar sua primeira transcrição.</p>
            </div>
          )}
        </div>
      </section>

    </div>
  );
}

function PipelineFact({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 px-5 first:pl-0 last:pr-0 max-[980px]:px-0 max-[980px]:py-3 max-[980px]:first:pt-0">
      <dt className="text-[11px] text-muted">{label}</dt>
      <dd className="mt-1 truncate text-[13px] font-medium text-ink" title={value}>{value}</dd>
    </div>
  );
}
