import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  BrainCircuit,
  ChevronDown,
  Clock3,
  Info,
  Loader2,
  Pause,
  Play,
} from "lucide-react";
import { Button } from "../components/ui/Button";
import { VoiceInsights } from "./VoiceInsights";
import { ErrorState, PageHeader, SkeletonRows } from "../components/ui/Surface";
import {
  adjacentInsightsTab,
  buildActivityCells,
  formatInsightNumber as number,
  type InsightsTab,
} from "./insights-utils";
import {
  getDevMode,
  getInsights,
  setInsightsBackfillPaused,
  type ApplicationInsight,
  type InsightPeriod,
  type InsightsResponse,
  type MetricTrend,
  type RankedCount,
} from "../lib/tauri";

type Tab = InsightsTab;

const PERIODS: Array<{ value: InsightPeriod; label: string }> = [
  { value: "today", label: "Hoje" },
  { value: "last7_days", label: "7 dias" },
  { value: "last30_days", label: "30 dias" },
  { value: "all_time", label: "Todo o período" },
];

const WEEKDAYS = ["segunda-feira", "terça-feira", "quarta-feira", "quinta-feira", "sexta-feira", "sábado", "domingo"];

function duration(milliseconds: number) {
  const minutes = Math.round(milliseconds / 60_000);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const remaining = minutes % 60;
  return remaining ? `${hours} h ${remaining} min` : `${hours} h`;
}

function signed(value: number, suffix = "%") {
  return `${value > 0 ? "+" : ""}${number(value, 1)}${suffix}`;
}

function MetricHelp({ label }: { label: string }) {
  return (
    <span className="inline-flex cursor-help align-middle text-[#85867f] outline-hidden focus-visible:ring-2 focus-visible:ring-[#777870]" title={label} aria-label={label} role="note" tabIndex={0}>
      <Info className="h-3.5 w-3.5" aria-hidden />
    </span>
  );
}

function TrendBadge({ trend, absolute = false }: { trend?: MetricTrend; absolute?: boolean }) {
  if (!trend) return <span className="text-[11px] text-muted">Amostra insuficiente para tendência</span>;
  const value = absolute || trend.change_percent == null ? signed(trend.change_absolute, "") : signed(trend.change_percent);
  return <span className="text-[11px] tabular-nums text-[#555650]">{value} vs. período anterior</span>;
}

function RankedRows({ items, empty = "Ainda não há dados suficientes." }: { items: RankedCount[]; empty?: string }) {
  if (!items.length) return <p className="py-8 text-[13px] text-muted">{empty}</p>;
  return (
    <div className="divider-list">
      {items.map((item) => (
        <div key={item.label} className="grid min-h-12 grid-cols-[minmax(0,1fr)_120px_48px] items-center gap-4 py-2.5">
          <span className="truncate text-[13px] text-[#343530]" title={item.label}>{item.label}</span>
          <span className="h-1.5 overflow-hidden rounded-full bg-[#e7e7e1]" aria-hidden>
            <span className="block h-full rounded-full bg-[#777870]" style={{ width: `${Math.max(3, item.percentage)}%` }} />
          </span>
          <span className="text-right font-mono text-[11px] tabular-nums text-muted">{number(item.percentage, 0)}%</span>
        </div>
      ))}
    </div>
  );
}

function ApplicationRows({ items }: { items: ApplicationInsight[] }) {
  const [expanded, setExpanded] = useState<string | null>(null);
  const listId = useRef(`insight-apps-${crypto.randomUUID()}`);
  if (!items.length) return <p className="py-8 text-[13px] text-muted">O aplicativo de destino não estava disponível nas gravações deste período.</p>;
  return (
    <div className="divider-list">
      {items.map((item, index) => {
        const open = expanded === item.name;
        const domainsId = `${listId.current}-${index}`;
        return (
          <div key={item.name} className="py-3">
            <button
              type="button"
              className="grid min-h-10 w-full grid-cols-[minmax(0,1fr)_150px_52px_18px] items-center gap-4 text-left"
              onClick={() => item.domains.length && setExpanded(open ? null : item.name)}
              disabled={!item.domains.length}
              aria-expanded={item.domains.length ? open : undefined}
              aria-controls={item.domains.length ? domainsId : undefined}
            >
              <span className="truncate text-[13px] font-medium text-[#343530]" title={item.name}>{item.name}</span>
              <span className="h-1.5 overflow-hidden rounded-full bg-[#e7e7e1]" aria-hidden>
                <span className="block h-full rounded-full bg-[#555650]" style={{ width: `${Math.max(3, item.percentage)}%` }} />
              </span>
              <span className="text-right font-mono text-[11px] tabular-nums text-muted">{number(item.percentage)}%</span>
              {item.domains.length ? <ChevronDown className={`h-4 w-4 text-muted transition-transform ${open ? "rotate-180" : ""}`} aria-hidden /> : <span />}
            </button>
            {open && (
              <div id={domainsId} className="ml-4 mt-3 border-l border-line pl-4">
                {item.domains.map((domain) => (
                  <div key={domain.label} className="flex items-center justify-between py-1.5 text-[13px] text-muted">
                    <span className="truncate" title={domain.label}>{domain.label}</span>
                    <span className="font-mono text-[11px] tabular-nums">{domain.count} ditados</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function ActivityCalendar({ activity }: { activity: InsightsResponse["temporal"]["activity"] }) {
  const cells = useMemo(() => buildActivityCells(activity), [activity]);
  const max = Math.max(1, ...cells.map((cell) => cell.count));
  const activeDays = cells.filter((cell) => cell.count > 0).length;
  const totalSessions = cells.reduce((total, cell) => total + cell.count, 0);
  return (
    <div className="mt-5 grid grid-flow-col grid-rows-7 gap-1" aria-label={`Atividade nos últimos 91 dias: ${totalSessions} ditados em ${activeDays} dias ativos`}>
      {cells.map((cell) => {
        const strength = cell.count / max;
        const background = cell.count === 0 ? "#ecece7" : strength > .66 ? "#4c4d48" : strength > .33 ? "#85867f" : "#b9bab3";
        return <span key={cell.key} className="aspect-square min-w-0 rounded-[3px]" style={{ background }} title={`${cell.key}: ${cell.count} ditados`} aria-hidden />;
      })}
    </div>
  );
}

function UsageTab({ data }: { data: InsightsResponse }) {
  const wpmTrend = data.trends.find((trend) => trend.metric === "speaking_speed_wpm");
  return (
    <div className="space-y-10">
      <section className="surface overflow-hidden">
        <div className="grid grid-cols-[1.35fr_1fr_1fr] max-[940px]:grid-cols-1">
          <div className="border-r border-line p-7 max-[940px]:border-b max-[940px]:border-r-0">
            <div className="flex items-center gap-1.5 text-[13px] text-muted">Velocidade média <MetricHelp label="Palavras divididas pelo tempo estimado de fala, excluindo silêncio detectado quando há áudio analisado." /></div>
            <div className="mt-4 flex items-baseline gap-2"><strong className="text-[42px] font-semibold tracking-[-0.045em] tabular-nums text-ink">{data.usage.average_wpm ? number(data.usage.average_wpm) : "—"}</strong><span className="text-[13px] text-muted">PPM</span></div>
            <div className="mt-2"><TrendBadge trend={wpmTrend} /></div>
            {data.usage.typical_wpm && <p className="mt-5 text-[13px] text-muted">Faixa típica {number(data.usage.typical_wpm[0])}–{number(data.usage.typical_wpm[1])} PPM</p>}
          </div>
          <div className="border-r border-line p-7 max-[940px]:border-b max-[940px]:border-r-0">
            <div className="text-[13px] text-muted">Correções manuais</div>
            <div className="mt-4 text-[32px] font-semibold tracking-[-0.03em] tabular-nums">{number(data.usage.manual_corrections)}</div>
            <p className="mt-2 text-[13px] leading-5 text-muted">{number(data.usage.vocabulary_corrections)} correções já incorporadas ao vocabulário.</p>
          </div>
          <div className="p-7">
            <div className="text-[13px] text-muted">Palavras ditadas</div>
            <div className="mt-4 text-[32px] font-semibold tracking-[-0.03em] tabular-nums">{number(data.usage.words)}</div>
            <p className="mt-2 text-[13px] leading-5 text-muted">{number(data.usage.sessions)} ditados · {duration(data.usage.audio_duration_ms)} de áudio</p>
          </div>
        </div>
      </section>

      <section className="grid grid-cols-[minmax(0,1.45fr)_minmax(280px,.75fr)] gap-8 max-[900px]:grid-cols-1">
        <div>
          <h2 className="section-title">Onde você dita</h2>
          <p className="section-description">Os lugares em que o ditado acompanha você.</p>
          <div className="mt-5 border-y border-line"><ApplicationRows items={data.application_details} /></div>
        </div>
        <div className="surface-subtle p-6">
          <div className="flex items-center gap-2 text-[13px] font-medium text-ink"><Clock3 className="h-4 w-4 text-muted" aria-hidden /> Ritmo de uso</div>
          <dl className="mt-5 space-y-5">
            <div><dt className="meta-label">Horário mais ativo</dt><dd className="mt-1 text-[17px] font-semibold">{data.temporal.peak_hour == null ? "Ainda desconhecido" : `${String(data.temporal.peak_hour).padStart(2, "0")}:00–${String((data.temporal.peak_hour + 1) % 24).padStart(2, "0")}:00`}</dd></div>
            <div><dt className="meta-label">Dia mais ativo</dt><dd className="mt-1 text-[14px] font-medium">{data.temporal.peak_weekday == null ? "Ainda desconhecido" : WEEKDAYS[data.temporal.peak_weekday]}</dd></div>
            <div className="grid grid-cols-2 gap-4 border-t border-[#d9d9d3] pt-5"><div><dt className="meta-label">Sequência atual</dt><dd className="mt-1 text-[17px] font-semibold tabular-nums">{data.temporal.current_streak_days} dias</dd></div><div><dt className="meta-label">Maior sequência</dt><dd className="mt-1 text-[17px] font-semibold tabular-nums">{data.temporal.longest_streak_days} dias</dd></div></div>
          </dl>
          <ActivityCalendar activity={data.temporal.activity} />
        </div>
      </section>

      <section className="grid grid-cols-2 gap-8 max-[860px]:grid-cols-1">
        <div>
          <h2 className="section-title">Tipos de uso</h2>
          <p className="section-description">Como o ditado participa do seu dia.</p>
          <div className="mt-4 border-y border-line"><RankedRows items={data.categories} /></div>
        </div>
        <div>
          <h2 className="section-title">Mudanças recentes</h2>
          <p className="section-description">O que mudou no seu jeito de usar o ditado.</p>
          <div className="mt-4 divide-y divide-line border-y border-line">
            {data.trends.length ? data.trends.map((trend) => <TrendRow key={trend.metric} trend={trend} />) : <p className="py-8 text-[13px] text-muted">Continue ditando para formar uma linha de base comparável.</p>}
          </div>
        </div>
      </section>
    </div>
  );
}

function TrendRow({ trend }: { trend: MetricTrend }) {
  const names: Record<string, string> = {
    speaking_speed_wpm: "Velocidade de fala",
    voice_level_lufs: "Nível de voz estimado",
    corrections_per_1000_words: "Correções / 1.000 palavras",
    fillers_per_1000_words: "Palavras de apoio / 1.000",
  };
  const unit = trend.metric === "voice_level_lufs" ? " LU" : trend.metric === "speaking_speed_wpm" ? " PPM" : "";
  return <div className="flex items-center justify-between gap-5 py-3.5"><span className="text-[13px] text-[#41423e]">{names[trend.metric] ?? trend.metric}</span><span className="font-mono text-[11px] tabular-nums text-muted">{number(trend.previous, 1)} → {number(trend.current, 1)}{unit} · {trend.change_percent == null ? signed(trend.change_absolute, unit) : signed(trend.change_percent)}</span></div>;
}

export function InsightsView() {
  const [tab, setTab] = useState<Tab>("voice");
  const [period, setPeriod] = useState<InsightPeriod>("last30_days");
  const [data, setData] = useState<InsightsResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [backfillBusy, setBackfillBusy] = useState(false);
  const [backfillError, setBackfillError] = useState<string | null>(null);
  const [developerMode, setDeveloperMode] = useState(false);
  const mountedRef = useRef(true);
  const periodRef = useRef(period);
  const reloadRunningRef = useRef(false);
  const reloadQueuedRef = useRef(false);
  const reload = useCallback(async () => {
    if (reloadRunningRef.current) {
      reloadQueuedRef.current = true;
      return;
    }
    reloadRunningRef.current = true;
    try {
      do {
        reloadQueuedRef.current = false;
        const requestedPeriod = periodRef.current;
        if (mountedRef.current) setError(null);
        try {
          const response = await getInsights(requestedPeriod);
          if (mountedRef.current && requestedPeriod === periodRef.current) setData(response);
        } catch (reason) {
          if (mountedRef.current) setError(String(reason));
        } finally {
          if (mountedRef.current) setLoading(false);
        }
      } while (reloadQueuedRef.current && mountedRef.current);
    } finally {
      reloadRunningRef.current = false;
    }
  }, []);
  useEffect(() => {
    mountedRef.current = true;
    void getDevMode().then((enabled) => { if (mountedRef.current) setDeveloperMode(enabled); }).catch(() => undefined);
    return () => { mountedRef.current = false; };
  }, []);
  useEffect(() => {
    periodRef.current = period;
    setLoading(true);
    void reload();
  }, [period, reload]);
  useEffect(() => {
    let disposed = false;
    const unlisten: Array<() => void> = [];
    void Promise.all([
      listen("insights-progress", () => void reload()),
      listen("insights-updated", () => void reload()),
    ]).then((listeners) => {
      if (disposed) listeners.forEach((listener) => listener());
      else unlisten.push(...listeners);
    });
    return () => { disposed = true; unlisten.forEach((listener) => listener()); };
  }, [reload]);
  const pauseBackfill = async () => {
    if (!data) return;
    setBackfillBusy(true); setBackfillError(null);
    try { await setInsightsBackfillPaused(!data.backfill.paused); await reload(); } catch (reason) { setBackfillError(String(reason)); } finally { setBackfillBusy(false); }
  };
  const switchTab = (next: Tab) => {
    setTab(next);
    requestAnimationFrame(() => document.getElementById(`insights-tab-${next}`)?.focus());
  };
  const handleTabKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight" && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    switchTab(adjacentInsightsTab(tab, event.key));
  };
  const hasData = !!data?.usage.sessions;
  return (
    <div>
      <PageHeader title="Insights" description="Seu jeito de falar, em poucas palavras." action={<div className="inline-flex flex-wrap rounded-[10px] border border-line bg-white p-1" aria-label="Período analisado">{PERIODS.map((item) => <button key={item.value} type="button" onClick={() => setPeriod(item.value)} aria-pressed={period === item.value} className={`h-8 rounded-[7px] px-3 text-[11px] font-medium transition-colors ${period === item.value ? "bg-[#242422] text-white" : "text-muted hover:bg-[#f0f0eb] hover:text-ink"}`}>{item.label}</button>)}</div>} />
      <div className="mb-8 flex border-b border-line" role="tablist" aria-label="Áreas de Insights"><button id="insights-tab-usage" type="button" role="tab" aria-selected={tab === "usage"} aria-controls="insights-panel-usage" tabIndex={tab === "usage" ? 0 : -1} onKeyDown={handleTabKeyDown} onClick={() => setTab("usage")} className={`relative px-1 pb-3 pr-6 text-[13px] font-medium ${tab === "usage" ? "text-ink after:absolute after:inset-x-0 after:-bottom-px after:h-px after:bg-ink" : "text-muted"}`}>Seu uso</button><button id="insights-tab-voice" type="button" role="tab" aria-selected={tab === "voice"} aria-controls="insights-panel-voice" tabIndex={tab === "voice" ? 0 : -1} onKeyDown={handleTabKeyDown} onClick={() => setTab("voice")} className={`relative px-6 pb-3 text-[13px] font-medium ${tab === "voice" ? "text-ink after:absolute after:inset-x-5 after:-bottom-px after:h-px after:bg-ink" : "text-muted"}`}>Sua voz</button></div>
      {data?.backfill.running && <div className="mb-7 flex items-center justify-between gap-5 rounded-[10px] bg-[#eeeeea] px-4 py-3" role="status" aria-live="polite"><div className="flex min-w-0 items-center gap-3">{data.backfill.paused ? <Pause className="h-4 w-4 text-muted" /> : <Loader2 className="h-4 w-4 animate-spin text-muted" />}<div className="min-w-0"><p className="text-[13px] font-medium text-[#42433f]">{data.backfill.paused ? "Análise do histórico pausada" : "Analisando seu histórico…"}</p><p className="mt-0.5 text-[11px] tabular-nums text-muted">{number(data.backfill.processed)} / {number(data.backfill.total)} gravações</p></div></div><Button size="sm" variant="ghost" disabled={backfillBusy} onClick={pauseBackfill}>{backfillBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : data.backfill.paused ? <Play className="h-3.5 w-3.5" /> : <Pause className="h-3.5 w-3.5" />}{data.backfill.paused ? "Continuar" : "Pausar"}</Button></div>}
      {backfillError && <ErrorState>{backfillError}</ErrorState>}
      {error && <ErrorState>{error}</ErrorState>}
      {loading && !data ? <div className="surface"><SkeletonRows count={5} /></div> : !hasData ? <div className="surface flex min-h-[360px] flex-col items-center justify-center px-8 text-center"><Activity className="h-6 w-6 text-[#8b8c85]" /><h2 className="mt-5 text-[17px] font-semibold">Seus Insights começam com o próximo ditado</h2><p className="mt-2 max-w-lg text-[13px] leading-5 text-muted">Use o Sonora no seu dia a dia. Suas primeiras descobertas aparecem aqui conforme você dita.</p></div> : data && <div id={`insights-panel-${tab}`} role="tabpanel" aria-labelledby={`insights-tab-${tab}`} tabIndex={0}>{tab === "usage" ? <UsageTab data={data} /> : <VoiceInsights data={data} reload={reload} developerMode={developerMode} />}</div>}
      {data && <footer className="mt-12 flex items-center justify-between border-t border-line pt-5 text-[11px] text-muted">{developerMode ? <span>Analysis version {data.analysis_version} · {number(data.audio.analyzed_sessions)} gravações com áudio analisado</span> : <span>Suas estatísticas ficam neste computador.</span>}<span className="inline-flex items-center gap-1.5"><BrainCircuit className="h-3.5 w-3.5" /> Estatísticas calculadas localmente</span></footer>}
    </div>
  );
}
