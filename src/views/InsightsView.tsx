import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Info,
  Loader2,
  Pause,
  Play,
} from "lucide-react";
import { Button } from "../components/ui/Button";
import { VoiceInsights } from "./VoiceInsights";
import { ErrorState, PageHeader, Segmented, SkeletonRows } from "../components/ui/Surface";
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
  { value: "all_time", label: "Tudo" },
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
    <span className="inline-flex cursor-help align-middle text-faint outline-hidden hover:text-muted focus-visible:ring-2 focus-visible:ring-ink" title={label} aria-label={label} role="note" tabIndex={0}>
      <Info className="h-3.5 w-3.5" aria-hidden />
    </span>
  );
}

function TrendBadge({ trend, absolute = false }: { trend?: MetricTrend; absolute?: boolean }) {
  if (!trend) return <span>sem base para comparar ainda</span>;
  const value = absolute || trend.change_percent == null ? signed(trend.change_absolute, "") : signed(trend.change_percent);
  return <span className="tabular-nums">{value} vs. período anterior</span>;
}

function Bar({ value, strong = false }: { value: number; strong?: boolean }) {
  return (
    <span className="h-1 overflow-hidden rounded-full bg-fill" aria-hidden>
      <span className={"block h-full rounded-full " + (strong ? "bg-soft" : "bg-faint")} style={{ width: `${Math.max(3, value)}%` }} />
    </span>
  );
}

function RankedRows({ items, empty = "Ainda não há dados suficientes." }: { items: RankedCount[]; empty?: string }) {
  if (!items.length) return <p className="py-6 text-[13px] text-muted">{empty}</p>;
  return (
    <div className="hairline-list">
      {items.map((item) => (
        <div key={item.label} className="grid min-h-11 grid-cols-[minmax(0,1fr)_96px_44px] items-center gap-4 py-2">
          <span className="truncate text-[13px] text-strong" title={item.label}>{item.label}</span>
          <Bar value={item.percentage} />
          <span className="timecode text-right">{number(item.percentage, 0)}%</span>
        </div>
      ))}
    </div>
  );
}

function ApplicationRows({ items }: { items: ApplicationInsight[] }) {
  if (!items.length) return <p className="py-6 text-[13px] text-muted">O aplicativo de destino não estava disponível nas gravações deste período.</p>;
  return (
    <div className="hairline-list">
      {items.map((item) => (
        <div key={item.name} className="grid min-h-12 grid-cols-[minmax(0,1fr)_120px_48px] items-center gap-4 py-2.5">
          <span className="min-w-0">
            <span className="block truncate text-[13px] font-medium text-ink" title={item.name}>{item.name}</span>
            {item.domains.length > 0 && <span className="block truncate text-[12px] text-muted">{item.domains.map((domain) => `${domain.label} (${domain.count})`).join(" · ")}</span>}
          </span>
          <Bar value={item.percentage} strong />
          <span className="timecode text-right">{number(item.percentage)}%</span>
        </div>
      ))}
    </div>
  );
}

function ActivityCalendar({ activity }: { activity: InsightsResponse["temporal"]["activity"] }) {
  const cells = useMemo(() => buildActivityCells(activity), [activity]);
  const max = Math.max(1, ...cells.map((cell) => cell.count));
  const activeDays = cells.filter((cell) => cell.count > 0).length;
  const totalSessions = cells.reduce((total, cell) => total + cell.count, 0);
  return (
    <div className="mt-6 grid max-w-[360px] grid-flow-col grid-rows-7 gap-[3px]" aria-label={`Atividade nos últimos 91 dias: ${totalSessions} ditados em ${activeDays} dias ativos`}>
      {cells.map((cell) => {
        const strength = cell.count / max;
        const background = cell.count === 0 ? "var(--n3)" : strength > .66 ? "var(--n9)" : strength > .33 ? "var(--n7)" : "var(--n5)";
        return <span key={cell.key} className="aspect-square min-w-0 rounded-[2px]" style={{ background }} title={`${cell.key}: ${cell.count} ditados`} aria-hidden />;
      })}
    </div>
  );
}

function Fact({ label, value, note }: { label: React.ReactNode; value: string; note?: React.ReactNode }) {
  return (
    <div className="grid min-h-12 grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-6 py-3">
      <dt className="flex items-center gap-1.5 text-[13px] text-strong">{label}</dt>
      <dd className="text-right font-mono text-[13px] font-medium tabular-nums text-ink">{value}</dd>
      {note && <dd className="col-span-2 mt-0.5 text-[12px] text-muted">{note}</dd>}
    </div>
  );
}

function UsageTab({ data }: { data: InsightsResponse }) {
  const wpmTrend = data.trends.find((trend) => trend.metric === "speaking_speed_wpm");
  return (
    <div>
      <section>
        <h2 className="section-title">Resumo do período</h2>
        <dl className="mt-3 hairline-list border-y border-line">
          <Fact
            label={<>Velocidade média <MetricHelp label="Palavras divididas pelo tempo estimado de fala, excluindo silêncio detectado quando há áudio analisado." /></>}
            value={data.usage.average_wpm ? `${number(data.usage.average_wpm)} PPM` : "—"}
            note={<><TrendBadge trend={wpmTrend} />{data.usage.typical_wpm && <> · faixa típica {number(data.usage.typical_wpm[0])}–{number(data.usage.typical_wpm[1])} PPM</>}</>}
          />
          <Fact label="Palavras ditadas" value={number(data.usage.words)} note={`${number(data.usage.sessions)} ditados · ${duration(data.usage.audio_duration_ms)} de áudio`} />
          <Fact label="Correções manuais" value={number(data.usage.manual_corrections)} note={`${number(data.usage.vocabulary_corrections)} já ${data.usage.vocabulary_corrections === 1 ? "incorporada" : "incorporadas"} ao vocabulário`} />
        </dl>
      </section>

      <section className="mt-12">
        <h2 className="section-title">Onde você dita</h2>
        <div className="mt-3 border-y border-line"><ApplicationRows items={data.application_details} /></div>
      </section>

      <section className="mt-12">
        <h2 className="section-title">Ritmo de uso</h2>
        <dl className="mt-4 grid grid-cols-4 gap-8 max-[820px]:grid-cols-2">
          <div><dt className="meta-label">Horário mais ativo</dt><dd className="mt-1 text-[14px] font-medium tabular-nums">{data.temporal.peak_hour == null ? "Ainda desconhecido" : `${String(data.temporal.peak_hour).padStart(2, "0")}:00–${String((data.temporal.peak_hour + 1) % 24).padStart(2, "0")}:00`}</dd></div>
          <div><dt className="meta-label">Dia mais ativo</dt><dd className="mt-1 text-[14px] font-medium">{data.temporal.peak_weekday == null ? "Ainda desconhecido" : WEEKDAYS[data.temporal.peak_weekday]}</dd></div>
          <div><dt className="meta-label">Sequência atual</dt><dd className="mt-1 text-[14px] font-medium tabular-nums">{data.temporal.current_streak_days} dias</dd></div>
          <div><dt className="meta-label">Maior sequência</dt><dd className="mt-1 text-[14px] font-medium tabular-nums">{data.temporal.longest_streak_days} dias</dd></div>
        </dl>
        <ActivityCalendar activity={data.temporal.activity} />
      </section>

      <section className="mt-12 grid grid-cols-2 gap-12 max-[820px]:grid-cols-1">
        <div>
          <h2 className="section-title">Tipos de uso</h2>
          <div className="mt-3 border-y border-line"><RankedRows items={data.categories} /></div>
        </div>
        <div>
          <h2 className="section-title">Mudanças recentes</h2>
          <div className="mt-3 hairline-list border-y border-line">
            {data.trends.length ? data.trends.map((trend) => <TrendRow key={trend.metric} trend={trend} />) : <p className="py-6 text-[13px] text-muted">Continue ditando para formar uma linha de base comparável.</p>}
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
  return <div className="flex min-h-11 items-center justify-between gap-5 py-2"><span className="text-[13px] text-strong">{names[trend.metric] ?? trend.metric}</span><span className="timecode">{number(trend.previous, 1)} → {number(trend.current, 1)}{unit} · {trend.change_percent == null ? signed(trend.change_absolute, unit) : signed(trend.change_percent)}</span></div>;
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
  const tabClass = (active: boolean) => `relative -mb-px h-10 border-b px-0.5 text-[13px] font-medium transition-colors ${active ? "border-ink text-ink" : "border-transparent text-muted hover:text-ink"}`;
  return (
    <div>
      <PageHeader
        title="Insights"
        action={<Segmented<InsightPeriod> label="Período analisado" value={period} onChange={setPeriod} options={PERIODS} />}
      />
      <div className="mb-10 flex gap-7 border-b border-line" role="tablist" aria-label="Áreas de Insights">
        <button id="insights-tab-voice" type="button" role="tab" aria-selected={tab === "voice"} aria-controls="insights-panel-voice" tabIndex={tab === "voice" ? 0 : -1} onKeyDown={handleTabKeyDown} onClick={() => setTab("voice")} className={tabClass(tab === "voice")}>Sua voz</button>
        <button id="insights-tab-usage" type="button" role="tab" aria-selected={tab === "usage"} aria-controls="insights-panel-usage" tabIndex={tab === "usage" ? 0 : -1} onKeyDown={handleTabKeyDown} onClick={() => setTab("usage")} className={tabClass(tab === "usage")}>Seu uso</button>
      </div>
      {data?.backfill.running && (
        <div className="mb-8 flex items-center justify-between gap-5 border-y border-line py-3" role="status" aria-live="polite">
          <div className="flex min-w-0 items-center gap-3">
            {data.backfill.paused ? <Pause className="h-4 w-4 text-muted" aria-hidden /> : <Loader2 className="h-4 w-4 animate-spin text-standby" aria-hidden />}
            <p className="text-[13px] text-ink">{data.backfill.paused ? "Análise do histórico pausada" : "Analisando seu histórico"} <span className="timecode ml-2">{number(data.backfill.processed)} / {number(data.backfill.total)}</span></p>
          </div>
          <Button size="sm" variant="ghost" disabled={backfillBusy} onClick={pauseBackfill}>{backfillBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> : data.backfill.paused ? <Play className="h-3.5 w-3.5" aria-hidden /> : <Pause className="h-3.5 w-3.5" aria-hidden />}{data.backfill.paused ? "Continuar" : "Pausar"}</Button>
        </div>
      )}
      {backfillError && <ErrorState>{backfillError}</ErrorState>}
      {error && <ErrorState>{error}</ErrorState>}
      {loading && !data ? <SkeletonRows count={5} /> : !hasData ? (
        <div className="py-10">
          <h2 className="text-[17px] font-semibold text-ink">Seus Insights começam com o próximo ditado</h2>
          <p className="mt-2 max-w-[52ch] text-[13px] leading-6 text-muted">Use o Sonora no dia a dia. As primeiras descobertas sobre sua fala e seu uso aparecem aqui conforme você dita.</p>
        </div>
      ) : data && <div id={`insights-panel-${tab}`} role="tabpanel" aria-labelledby={`insights-tab-${tab}`} tabIndex={0} className="outline-none">{tab === "usage" ? <UsageTab data={data} /> : <VoiceInsights data={data} reload={reload} developerMode={developerMode} />}</div>}
      {data && <footer className="mt-16 flex items-center justify-between gap-4 border-t border-line pt-5 text-[11.5px] text-muted">{developerMode ? <span className="font-mono">analysis v{data.analysis_version} · {number(data.audio.analyzed_sessions)} gravações com áudio analisado</span> : <span>Calculado localmente. Suas estatísticas ficam neste computador.</span>}</footer>}
    </div>
  );
}
