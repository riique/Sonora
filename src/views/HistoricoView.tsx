import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Braces,
  Check,
  ClipboardCopy,
  Copy,
  FileAudio,
  FolderOpen,
  Loader2,
  Pause,
  Pencil,
  Play,
  RefreshCw,
  RotateCcw,
  Search,
  Sparkles,
  Trash2,
  Undo2,
  X,
} from "lucide-react";
import { Button } from "../components/ui/Button";
import { Input, Textarea } from "../components/ui/Input";
import { Menu } from "../components/ui/Menu";
import { LOG_GRID, LogStamp } from "../components/ui/LogRow";
import { EmptyState, ErrorState, PageHeader, Segmented, SkeletonRows } from "../components/ui/Surface";
import { PronunciationEvaluation } from "../components/PronunciationEvaluation";
import { clockTime, dayKey, dayLabel, formatDuration, parseEntryDate } from "../lib/format";
import {
  evaluatePronunciation,
  getDevMode,
  getHistoryDetail,
  getHistoryPage,
  readHistoryAudio,
  revealHistoryAudio,
  retryTranscription,
  retryTranscriptionWithFallback,
  undoAiEdit,
  type HistoryEntry,
  type PipelineRun,
} from "../lib/tauri";
import { ScratchpadView } from "./ScratchpadView";
import { TranscricaoView } from "./TranscricaoView";
import type { Navigate } from "./index";

const PAGE_SIZE = 50;

function productModeLabel(mode: string | null | undefined): string | null {
  if (!mode) return null;
  return ({
    "ultra-fast": "Ultrarrápido",
    "fast-accurate": "Rápido e preciso",
    precise: "Preciso",
    "ultra-precise": "Ultrapreciso",
  } as Record<string, string>)[mode] ?? mode;
}

function audioMimeType(path: string | null | undefined): string {
  const extension = path?.split(".").pop()?.toLowerCase();
  return ({
    wav: "audio/wav",
    wave: "audio/wav",
    mp3: "audio/mpeg",
    m4a: "audio/mp4",
    mp4: "audio/mp4",
    aac: "audio/mp4",
    flac: "audio/flac",
    ogg: "audio/ogg",
    oga: "audio/ogg",
    webm: "audio/webm",
  } as Record<string, string>)[extension ?? ""] ?? "application/octet-stream";
}

function formatHistoryForClipboard(items: HistoryEntry[]): string {
  return items
    .filter((item) => !item.is_error && item.text.trim())
    .map((item, index) => {
      const model = item.model?.trim() || item.engine?.trim() || "Não informado";
      const pipeline = productModeLabel(item.mode) || "Legado";
      const stages = item.stages?.split(",").map((stage) => stage.trim()).filter(Boolean).join(" → ");
      return [
        `=== Transcrição ${index + 1} ===`,
        `Data: ${item.date}`,
        `Modelo: ${model}`,
        `Pipeline: ${pipeline}`,
        ...(stages ? [`Etapas: ${stages}`] : []),
        "",
        item.text.trim(),
      ].join("\n");
    })
    .join("\n\n");
}

function milliseconds(value: number | null | undefined): string {
  if (value == null) return "Desconhecido";
  return value < 1000 ? `${value} ms` : `${(value / 1000).toFixed(2)} s`;
}

function costLabel(run: PipelineRun): string {
  const cost = run.usage?.cost;
  if (!cost || cost.kind === "unknown" || cost.amount_usd == null) return "Desconhecido";
  const origin = cost.kind === "actual" ? "real" : "estimado";
  return `$${cost.amount_usd.toFixed(6)} (${origin})`;
}

function transcriptRows(run: PipelineRun): Array<[string, string]> {
  const labels: Array<[keyof PipelineRun["transcript"], string]> = [
    ["raw", "Bruta"],
    ["refined", "Refinada"],
    ["formatted", "Formatada"],
    ["delivered", "Entregue"],
    ["user_corrected", "Corrigida por você"],
  ];
  return labels.flatMap(([key, label]) => {
    const value = run.transcript?.[key];
    return value == null ? [] : [[label, value] as [string, string]];
  });
}

function redactTechnicalText(value: unknown): string {
  const serialized = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  return serialized
    .replace(/Bearer\s+[^\s"']+/gi, "Bearer [REDACTED]")
    .replace(/("(?:api[_-]?key|authorization|token)"\s*:\s*")[^"]+("?)/gi, "$1[REDACTED]$2")
    .replace(/\b(?:sk|gsk|AIza)[-_A-Za-z0-9]{16,}\b/g, "[REDACTED]");
}

function Disclosure({ title, children, open = false }: { title: string; children: React.ReactNode; open?: boolean }) {
  return (
    <details className="group/d mt-5" open={open}>
      <summary className="inline-flex cursor-pointer items-center gap-1.5 text-[12px] font-medium text-muted hover:text-ink">
        <span className="inline-block h-1.5 w-1.5 -rotate-45 border-b-[1.5px] border-r-[1.5px] border-current transition-transform group-open/d:rotate-45" aria-hidden />
        {title}
      </summary>
      <div className="mt-3">{children}</div>
    </details>
  );
}

function PipelineInspector({ entry, devMode }: { entry: HistoryEntry; devMode: boolean }) {
  const runs = entry.pipeline_runs ?? [];
  if (!runs.length) {
    return (
      <p className="mt-4 text-[12px] text-muted">
        Esta entrada ainda usa o formato histórico legado. Ela será migrada automaticamente na próxima gravação do histórico.
      </p>
    );
  }

  return (
    <div className="mt-5 space-y-6 border-l border-line pl-5">
      {runs.map((run, runIndex) => {
        const versions = transcriptRows(run);
        const finalAttempt = [...run.attempts].reverse().find((attempt) => attempt.status === "success");
        return (
          <section key={run.id}>
            <div className="flex items-center justify-between gap-3">
              <h4 className="text-[12px] font-semibold text-ink">Execução {runIndex + 1}</h4>
              <span className="timecode uppercase">{run.status}</span>
            </div>

            <dl className="mt-3 grid grid-cols-2 gap-x-8 gap-y-3 sm:grid-cols-4">
              {[
                ["Tempo total", milliseconds(run.timings?.total_ms)],
                ["Provedor", finalAttempt?.provider || "Desconhecido"],
                ["Modelo", finalAttempt?.model || entry.model || entry.engine],
                ["Fallback", run.fallback?.used ? "Sim" : "Não"],
                ["Tokens", run.usage?.total_tokens?.toString() || "Desconhecido"],
                ["Custo", costLabel(run)],
                ["Style", run.profile_id || "Padrão"],
                ["Saída", `${run.formatting_level} · ${run.destination}`],
              ].map(([label, value]) => (
                <div key={label} className="min-w-0">
                  <dt className="meta-label">{label}</dt>
                  <dd className="mt-0.5 truncate font-mono text-[11.5px] text-strong" title={value}>{value}</dd>
                </div>
              ))}
            </dl>

            {run.fallback?.used && (
              <p className="mt-4 text-[12px] leading-5 text-standby">
                Fallback utilizado{run.fallback.reason ? `: ${run.fallback.reason}` : "."}
              </p>
            )}

            {versions.length > 0 && (
              <Disclosure title="Versões da transcrição" open>
                <dl className="space-y-3">
                  {versions.map(([label, value]) => (
                    <div key={label}>
                      <dt className="meta-label">{label}</dt>
                      <dd className="mt-0.5 max-w-[68ch] whitespace-pre-wrap text-[12.5px] leading-5 text-strong">{value}</dd>
                    </div>
                  ))}
                </dl>
              </Disclosure>
            )}

            {devMode && (
              <>
                <Disclosure title={`Tentativas (${run.attempts.length})`}>
                  <div className="hairline-list border-y border-line">
                    {run.attempts.map((attempt) => (
                      <div key={attempt.id} className="grid grid-cols-[minmax(0,1fr)_auto] gap-3 py-2 text-[11.5px]">
                        <div><span className="font-medium">{attempt.provider}</span><span className="font-mono text-muted"> / {attempt.model}</span><div className="mt-0.5 text-muted">{attempt.transport}{attempt.usage?.bytes_sent != null ? ` · ${attempt.usage.bytes_sent} bytes` : ""}{attempt.usage?.total_tokens != null ? ` · ${attempt.usage.total_tokens} tokens` : ""}{attempt.error ? ` · ${attempt.error.message}` : ""}</div></div>
                        <div className="text-right font-mono"><div>{attempt.status.toUpperCase()}</div><div className="text-muted">{milliseconds(attempt.duration_ms)}</div></div>
                      </div>
                    ))}
                  </div>
                </Disclosure>

                <Disclosure title="Tempos detalhados">
                  <div className="grid grid-cols-2 gap-x-8 gap-y-1.5 sm:grid-cols-3">
                    {Object.entries(run.timings ?? {}).filter(([, value]) => value != null).map(([name, value]) => (
                      <div key={name} className="flex items-center justify-between gap-2 text-[11px]"><span className="text-muted">{name}</span><span className="font-mono">{milliseconds(Number(value))}</span></div>
                    ))}
                  </div>
                </Disclosure>

                <Disclosure title={`Etapas (${run.stages.length})`}>
                  <div className="hairline-list border-y border-line">
                    {run.stages.map((stage) => (
                      <div key={stage.id} className="flex items-center justify-between gap-3 py-2 text-[11.5px]"><span>{stage.stage.replace(/_/g, " ")}</span><span className="font-mono text-muted">{milliseconds(stage.duration_ms)}</span></div>
                    ))}
                  </div>
                </Disclosure>

                {(run.debug_info || run.attempts.some((attempt) => attempt.result.request_sanitized || attempt.result.response_sanitized)) && (
                  <Disclosure title="Requisição e resposta (sanitizadas)">
                    <pre className="scrollbar-thin max-h-72 overflow-auto whitespace-pre-wrap wrap-break-word rounded-[8px] bg-fill p-3 font-mono text-[10.5px] leading-4 text-strong">
                      {redactTechnicalText(run.attempts.map((attempt) => ({ provider: attempt.provider, request: attempt.result.request_sanitized, response: attempt.result.response_sanitized })))}
                    </pre>
                  </Disclosure>
                )}
              </>
            )}
          </section>
        );
      })}
    </div>
  );
}

type Tab = "ditados" | "notas";

export function HistoricoView({ onNavigate }: { onNavigate?: Navigate } = {}) {
  const [tab, setTab] = useState<Tab>("ditados");
  const [uploadOpen, setUploadOpen] = useState(false);
  const [offset, setOffset] = useState(0);
  const [total, setTotal] = useState(0);
  const generation = useRef(0);
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [devMode, setDevMode] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState<Record<string, boolean>>({});
  const [evaluationOpen, setEvaluationOpen] = useState<Record<string, boolean>>({});
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [loadingAudioId, setLoadingAudioId] = useState<string | null>(null);
  const [playingId, setPlayingId] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const [errors, setErrors] = useState<Record<string, string>>({});
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const audioUrlRef = useRef<string | null>(null);
  const activeAudioIdRef = useRef<string | null>(null);

  const releaseAudio = () => {
    const audio = audioRef.current;
    if (audio) {
      audio.pause();
      audio.removeAttribute("src");
      audio.load();
    }
    if (audioUrlRef.current) URL.revokeObjectURL(audioUrlRef.current);
    audioRef.current = null;
    audioUrlRef.current = null;
    activeAudioIdRef.current = null;
  };

  const disposeAudio = () => {
    releaseAudio();
    setPlayingId(null);
  };

  const setEntryError = (id: string, message: string) => setErrors((current) => ({ ...current, [id]: message }));

  const refresh = useCallback(async () => {
    const request = ++generation.current;
    try {
      const page = await getHistoryPage(query, offset);
      if (request !== generation.current) return;
      setItems(page.items);
      setTotal(page.total);
      setErrors((current) => ({ ...current, load: "" }));
    } catch (error) {
      setErrors((current) => ({ ...current, load: `Não foi possível carregar o histórico: ${String(error)}` }));
    } finally {
      setLoading(false);
    }
  }, [query, offset]);

  useEffect(() => {
    void refresh();
    void getDevMode().then(setDevMode).catch(console.error);
    const unlistenPromise = listen("transcription-saved", () => void refresh());
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
      releaseAudio();
    };
  }, [refresh]);

  const inspect = async (entry: HistoryEntry) => {
    if (detailsOpen[entry.id]) {
      setDetailsOpen((current) => ({ ...current, [entry.id]: false }));
      return;
    }
    try {
      const detail = await getHistoryDetail(entry.id);
      setItems((current) => current.map((item) => (item.id === entry.id ? detail : item)));
      setDetailsOpen((current) => ({ ...current, [entry.id]: true }));
    } catch (e) {
      setEntryError(entry.id, String(e));
    }
  };

  const copyText = async (id: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedId(id);
      window.setTimeout(() => setCopiedId((current) => (current === id ? null : current)), 1500);
    } catch {
      setEntryError(id, "Não foi possível copiar.");
    }
  };

  const toggleAudio = async (entry: HistoryEntry) => {
    const currentAudio = audioRef.current;
    if (activeAudioIdRef.current === entry.id && currentAudio) {
      if (currentAudio.paused) await currentAudio.play();
      else currentAudio.pause();
      return;
    }

    disposeAudio();
    setLoadingAudioId(entry.id);
    setErrors((current) => {
      const next = { ...current };
      delete next[entry.id];
      return next;
    });
    try {
      const bytes = await readHistoryAudio(entry.id);
      const url = URL.createObjectURL(new Blob([bytes], { type: audioMimeType(entry.audio_path) }));
      const audio = new Audio(url);
      audioRef.current = audio;
      audioUrlRef.current = url;
      activeAudioIdRef.current = entry.id;
      audio.onplay = () => setPlayingId(entry.id);
      audio.onpause = () => setPlayingId((current) => (current === entry.id ? null : current));
      audio.onended = disposeAudio;
      audio.onerror = () => {
        setEntryError(entry.id, "Não foi possível reproduzir o áudio salvo.");
        disposeAudio();
      };
      await audio.play();
    } catch (error) {
      disposeAudio();
      setEntryError(entry.id, String(error));
    } finally {
      setLoadingAudioId((current) => (current === entry.id ? null : current));
    }
  };

  const saveEdit = async (id: string) => {
    try {
      await invoke("update_history_text", { id, text: editDraft });
      setItems((current) => current.map((item) => (item.id === id ? { ...item, text: editDraft, words: editDraft.trim() ? editDraft.trim().split(/\s+/).length : 0, is_error: false, error_message: null } : item)));
      setEditingId(null);
    } catch (error) {
      setEntryError(id, String(error));
    }
  };

  const retry = async (id: string, withFallback = false) => {
    setBusyId(id);
    try {
      if (withFallback) await retryTranscriptionWithFallback(id);
      else await retryTranscription(id);
      await refresh();
    } catch (error) {
      setEntryError(id, String(error));
    } finally {
      setBusyId(null);
    }
  };

  const undo = async (entry: HistoryEntry, version: "raw" | "refined") => {
    try {
      const outcome = await undoAiEdit(entry.id, version);
      if (outcome === "copied_to_clipboard") setCopiedId(entry.id);
    } catch (error) {
      setEntryError(entry.id, String(error));
    }
  };

  const evaluate = async (entry: HistoryEntry) => {
    if (entry.evaluation) {
      setEvaluationOpen((current) => ({ ...current, [entry.id]: !current[entry.id] }));
      return;
    }
    setBusyId(entry.id);
    try {
      const evaluation = await evaluatePronunciation(entry.id);
      setItems((current) => current.map((item) => (item.id === entry.id ? { ...item, evaluation } : item)));
      setEvaluationOpen((current) => ({ ...current, [entry.id]: true }));
    } catch (error) {
      setEntryError(entry.id, String(error));
    } finally {
      setBusyId(null);
    }
  };

  const remove = async (id: string) => {
    if (!window.confirm("Excluir esta transcrição? Ela continua recuperável em Ajustes › Dados e recuperação.")) return;
    try {
      if (activeAudioIdRef.current === id) disposeAudio();
      await invoke("delete_history_entry", { id });
      setItems((current) => current.filter((item) => item.id !== id));
      setTotal((current) => Math.max(0, current - 1));
    } catch (error) {
      setEntryError(id, String(error));
    }
  };

  const copyAll = async () => {
    const text = formatHistoryForClipboard(items);
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setNotice("Transcrições desta página copiadas.");
    } catch {
      setNotice("Não foi possível copiar as transcrições.");
    }
    window.setTimeout(() => setNotice(""), 2400);
  };

  const clearAll = async () => {
    if (!window.confirm("Limpar todo o histórico e remover os áudios salvos?")) return;
    disposeAudio();
    try {
      await invoke("clear_history");
      setItems([]);
      setTotal(0);
    } catch (error) {
      setErrors((current) => ({ ...current, load: String(error) }));
    }
  };

  const hasCopyable = items.some((item) => !item.is_error && item.text.trim());

  // Group the page by calendar day; the day header replaces a date on every row.
  const groups: { key: string; label: string; entries: HistoryEntry[] }[] = [];
  for (const entry of items) {
    const date = parseEntryDate(entry.date);
    const key = date ? dayKey(date) : entry.date;
    const last = groups[groups.length - 1];
    if (last?.key === key) last.entries.push(entry);
    else groups.push({ key, label: date ? dayLabel(date) : entry.date, entries: [entry] });
  }

  return (
    <div>
      <PageHeader
        title="Histórico"
        action={tab === "ditados" && (
          <>
            <Button variant={uploadOpen ? "primary" : "secondary"} onClick={() => setUploadOpen((open) => !open)} aria-expanded={uploadOpen}>
              {uploadOpen ? <X className="h-4 w-4" aria-hidden /> : <FileAudio className="h-4 w-4" aria-hidden />}
              {uploadOpen ? "Fechar" : "Transcrever arquivo"}
            </Button>
            <Menu
              label="Mais opções do histórico"
              actions={[
                { label: "Copiar esta página", icon: <ClipboardCopy />, onSelect: () => void copyAll(), disabled: !hasCopyable },
                { label: "Itens removidos", icon: <Undo2 />, onSelect: () => onNavigate?.("ajustes", "dados"), hidden: !onNavigate },
                { label: "Limpar histórico", icon: <Trash2 />, onSelect: () => void clearAll(), danger: true, disabled: !items.length },
              ]}
            />
          </>
        )}
      />

      {uploadOpen && tab === "ditados" && (
        <div className="mb-10 animate-fade-in">
          <TranscricaoView />
        </div>
      )}

      <div className="mb-6 flex flex-wrap items-center gap-3">
        <Segmented<Tab>
          label="Tipo de conteúdo"
          value={tab}
          onChange={setTab}
          options={[{ value: "ditados", label: "Ditados" }, { value: "notas", label: "Notas" }]}
        />
        {tab === "ditados" && (
          <div className="relative min-w-56 flex-1">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted" aria-hidden />
            <Input
              type="search"
              aria-label="Buscar transcrições"
              placeholder="Buscar transcrições"
              value={query}
              onChange={(event) => {
                setOffset(0);
                setQuery(event.target.value);
              }}
              className="pl-9"
            />
          </div>
        )}
      </div>

      {tab === "notas" ? <ScratchpadView embedded /> : (
        <>
          {notice && <p role="status" className="mb-4 text-[12.5px] text-muted">{notice}</p>}
          {errors.load && <ErrorState><p>{errors.load}</p><button type="button" className="mt-1 font-medium underline" onClick={() => void refresh()}>Tentar novamente</button></ErrorState>}
          {loading ? (
            <SkeletonRows count={6} />
          ) : items.length === 0 ? (
            <div className="border-t border-line">
              <EmptyState
                title={query ? "Nada encontrado" : "Histórico vazio"}
                description={query ? "Tente buscar por outro trecho da fala." : "Seus ditados aparecem aqui, agrupados por dia. Você também pode transcrever um arquivo de áudio."}
              />
            </div>
          ) : (
            <div>
              {groups.map((group) => (
                <Fragment key={group.key}>
                  <h2 className="mt-10 flex items-baseline justify-between border-b border-line pb-2 first:mt-0">
                    <span className="text-[12.5px] font-semibold text-ink">{group.label}</span>
                    <span className="meta-label tabular-nums">{group.entries.length} {group.entries.length === 1 ? "ditado" : "ditados"}</span>
                  </h2>
                  <ul className="hairline-list">
                    {group.entries.map((entry) => {
                      const isError = Boolean(entry.is_error);
                      const hasAudio = Boolean(entry.audio_path);
                      const isBusy = busyId === entry.id;
                      const isPlaying = playingId === entry.id;
                      const latestRun = entry.pipeline_runs?.[entry.pipeline_runs.length - 1];
                      const date = parseEntryDate(entry.date);
                      return (
                        <li key={entry.id} className="group relative py-4">
                          <div className={LOG_GRID}>
                            <LogStamp time={date ? clockTime(date) : entry.date} duration={formatDuration(entry.duration_ms)} />

                            <div className="min-w-0 max-w-[68ch]">
                              {editingId === entry.id ? (
                                <div className="space-y-3">
                                  <Textarea autoFocus value={editDraft} onChange={(event) => setEditDraft(event.target.value)} className="min-h-28" aria-label="Editar transcrição" />
                                  <div className="flex gap-2">
                                    <Button size="sm" variant="primary" onClick={() => void saveEdit(entry.id)}>Salvar</Button>
                                    <Button size="sm" variant="ghost" onClick={() => setEditingId(null)}>Cancelar</Button>
                                  </div>
                                </div>
                              ) : isError ? (
                                <div>
                                  <h3 className="text-[13px] font-medium text-live">Falha na transcrição</h3>
                                  <p className="mt-0.5 text-[12.5px] leading-5 text-muted">{entry.error_message || "Não foi possível processar o áudio."}</p>
                                  {hasAudio && (
                                    <Button className="mt-3" size="sm" disabled={busyId !== null} onClick={() => void retry(entry.id)}>
                                      {isBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> : <RefreshCw className="h-3.5 w-3.5" aria-hidden />}
                                      Tentar de novo
                                    </Button>
                                  )}
                                </div>
                              ) : (
                                <>
                                  <p className="line-clamp-3 text-[13.5px] leading-[1.6] text-ink">{entry.text}</p>
                                  <p className="mt-1.5 text-[11.5px] text-muted">
                                    {entry.words} palavras · {productModeLabel(entry.mode) || entry.engine}
                                    {entry.used_fallback && <span className="text-standby"> · com fallback</span>}
                                    {devMode && entry.model && <span className="font-mono"> · {entry.model}</span>}
                                  </p>
                                </>
                              )}
                            </div>

                            <div className="flex items-start justify-end gap-0.5">
                              {hasAudio && (
                                <button
                                  type="button"
                                  className={"icon-button " + (isPlaying || loadingAudioId === entry.id ? "text-ink" : "reveal-on-row")}
                                  disabled={loadingAudioId === entry.id}
                                  onClick={() => void toggleAudio(entry)}
                                  aria-label={isPlaying ? "Pausar áudio" : "Reproduzir áudio"}
                                  title={isPlaying ? "Pausar áudio" : "Reproduzir áudio"}
                                >
                                  {loadingAudioId === entry.id ? <Loader2 className="h-4 w-4 animate-spin" /> : isPlaying ? <Pause className="h-4 w-4" /> : <Play className="h-4 w-4" />}
                                </button>
                              )}
                              {!isError && (
                                <button
                                  type="button"
                                  className={"icon-button " + (copiedId === entry.id ? "text-cue" : "reveal-on-row")}
                                  onClick={() => void copyText(entry.id, entry.text)}
                                  aria-label="Copiar transcrição"
                                  title="Copiar"
                                >
                                  {copiedId === entry.id ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                                </button>
                              )}
                              <div className={isBusy && !isError ? "" : "reveal-on-row focus-within:opacity-100 has-[[aria-expanded=true]]:opacity-100"}>
                                {isBusy && !isError ? (
                                  <span className="icon-button" aria-label="Processando"><Loader2 className="h-4 w-4 animate-spin" /></span>
                                ) : (
                                  <Menu
                                    label="Mais ações"
                                    actions={[
                                      { label: "Editar texto", icon: <Pencil />, onSelect: () => { setEditingId(entry.id); setEditDraft(entry.text); }, hidden: isError },
                                      { label: "Retranscrever", icon: <RefreshCw />, onSelect: () => void retry(entry.id), hidden: !hasAudio || isError, disabled: busyId !== null },
                                      { label: "Retranscrever com fallback", icon: <RefreshCw />, onSelect: () => void retry(entry.id, true), hidden: !hasAudio, disabled: busyId !== null },
                                      { label: entry.evaluation ? (evaluationOpen[entry.id] ? "Ocultar pronúncia" : "Ver pronúncia") : "Avaliar pronúncia", icon: <Sparkles />, onSelect: () => void evaluate(entry), hidden: !hasAudio || isError },
                                      { label: "Voltar à versão bruta", icon: <RotateCcw />, onSelect: () => void undo(entry, "raw"), hidden: isError || !latestRun?.transcript.raw },
                                      { label: "Voltar à versão refinada", icon: <RotateCcw />, onSelect: () => void undo(entry, "refined"), hidden: isError || !latestRun?.transcript.refined },
                                      { label: "Mostrar áudio na pasta", icon: <FolderOpen />, onSelect: () => void revealHistoryAudio(entry.id).catch((error) => setEntryError(entry.id, String(error))), hidden: !hasAudio },
                                      { label: detailsOpen[entry.id] ? "Ocultar detalhes técnicos" : "Detalhes técnicos", icon: <Braces />, onSelect: () => void inspect(entry) },
                                      { label: "Excluir", icon: <Trash2 />, onSelect: () => void remove(entry.id), danger: true },
                                    ]}
                                  />
                                )}
                              </div>
                            </div>
                          </div>

                          <div className="ml-[72px] max-w-[68ch]">
                            {entry.error_message && !isError && <p role="status" className="mt-3 text-[12.5px] text-live">{entry.error_message} Use Copiar para entregar o texto sem retranscrever.</p>}
                            {errors[entry.id] && <p className="mt-3 text-[12px] text-live" role="alert">{errors[entry.id]}</p>}
                            {detailsOpen[entry.id] && <PipelineInspector entry={entry} devMode={devMode} />}
                            {evaluationOpen[entry.id] && entry.evaluation && <div className="mt-5"><PronunciationEvaluation markdown={entry.evaluation} /></div>}
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                </Fragment>
              ))}

              {total > PAGE_SIZE && (
                <nav aria-label="Paginação do histórico" className="mt-10 flex items-center justify-between border-t border-line pt-5">
                  <span className="meta-label tabular-nums" aria-live="polite">
                    {offset + 1}–{Math.min(offset + items.length, total)} de {total.toLocaleString("pt-BR")}
                  </span>
                  <div className="flex gap-2">
                    <Button size="sm" variant="ghost" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}>Anteriores</Button>
                    <Button size="sm" disabled={offset + PAGE_SIZE >= total} onClick={() => setOffset(offset + PAGE_SIZE)}>Mais antigos</Button>
                  </div>
                </nav>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}
