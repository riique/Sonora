import { useCallback, useEffect, useRef, useState } from "react";
import { AlertCircle, Check, Mic, MicOff, RefreshCw, Square, X } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import {
  acknowledgeGadgetRendered,
  cancelRecording,
  getRecordingStatus,
  getShortcuts,
  getWidgetPreferences,
  retryTranscription,
  setGadgetVisualState,
  toggleRecordingState,
  type HistoryEntry,
  type GadgetPresentation,
  type PipelineProgressEvent,
  type RecordingStatus,
  type WidgetPreferences,
  type WidgetVisibilityMode,
} from "../lib/tauri";
import {
  GADGET_STATES,
  restState,
  showsProcessingLabel,
  stateAfterTimeout,
  type GadgetState,
} from "../gadget/machine";
import {
  belongsToRecordingSession,
  shouldApplyRecordingStatus,
} from "../recording/status";

const BAR_COUNT = 18;
const WAVE_WEIGHTS = [
  0.18, 0.28, 0.42, 0.6, 0.78, 0.92, 0.7, 0.48, 0.82,
  1, 0.76, 0.58, 0.88, 0.72, 0.54, 0.38, 0.26, 0.18,
];

function shortcutLabel(value: string): string {
  return value.replace("CommandOrControl", "Ctrl").replace("Control", "Ctrl").split("+").join(" + ");
}

export function GadgetApp() {
  const [state, setState] = useState<GadgetState>("hidden");
  const [visibilityMode, setVisibilityMode] = useState<WidgetVisibilityMode>("auto");
  const [audioLevel, setAudioLevel] = useState(0);
  const [shortcut, setShortcut] = useState("Ctrl + B");
  const [failure, setFailure] = useState<{ id: string; message: string; canRetry: boolean } | null>(null);
  const [retrying, setRetrying] = useState(false);
  const [progressMessage, setProgressMessage] = useState<string | null>(null);
  const pillRef = useRef<HTMLDivElement>(null);
  const operationRef = useRef(0);
  const stateRef = useRef<GadgetState>("hidden");
  const modeRef = useRef<WidgetVisibilityMode>("auto");
  const presentationRef = useRef<GadgetPresentation | null>(null);
  const lastRenderAckRef = useRef("");
  const visualStateQueueRef = useRef<Promise<void>>(Promise.resolve());
  const latestRecordingRevisionRef = useRef(-1);
  const activeSessionIdRef = useRef<string | null>(null);

  const reportRendered = useCallback((force = false) => {
    const element = pillRef.current;
    const presentation = presentationRef.current;
    if (
      !element
      || !presentation
      || GADGET_STATES[stateRef.current].visibility === "hidden"
      || presentation.visual_state !== stateRef.current
    ) return;

    const rect = element.getBoundingClientRect();
    const parentRect = element.parentElement?.getBoundingClientRect();
    if (!parentRect) return;
    // getBoundingClientRect includes the reveal/morph transform. Preserve the
    // layout footprint used by native hit-testing while the pill animates.
    const width = element.offsetWidth;
    const height = element.offsetHeight;
    const measured = {
      x: rect.left + rect.width / 2 - width / 2 - parentRect.left,
      y: rect.top + rect.height / 2 - height / 2 - parentRect.top,
      width,
      height,
    };
    const acknowledgementKey = `${presentation.generation}:${JSON.stringify(measured)}`;
    if (!force && acknowledgementKey === lastRenderAckRef.current) return;
    lastRenderAckRef.current = acknowledgementKey;
    void acknowledgeGadgetRendered(presentation, measured)
      .then((accepted) => {
        if (!accepted && lastRenderAckRef.current === acknowledgementKey) {
          lastRenderAckRef.current = "";
        }
      })
      .catch((error) => {
        if (lastRenderAckRef.current === acknowledgementKey) lastRenderAckRef.current = "";
        console.error("acknowledge_gadget_rendered failed:", error);
      });
  }, []);

  const transition = useCallback((next: GadgetState) => {
    if (stateRef.current === next) return;
    stateRef.current = next;
    setState(next);
    visualStateQueueRef.current = visualStateQueueRef.current
      .catch(() => undefined)
      .then(async () => {
        // A newer transition already owns presentation. Skipping stale queue
        // entries prevents duplicate native resizes from resetting input state.
        if (stateRef.current !== next) return;
        const presentation = await setGadgetVisualState(next);
        if (stateRef.current !== next) return;
        presentationRef.current = presentation;
        if (presentation.visual_state !== next) {
          stateRef.current = presentation.visual_state;
          setState(presentation.visual_state);
        }
        if (presentation.visual_state === "hidden") {
          lastRenderAckRef.current = "";
          return;
        }
        window.requestAnimationFrame(() => {
          window.requestAnimationFrame(() => reportRendered(true));
        });
      })
      .catch((error) => console.error("set_gadget_visual_state failed:", error));
  }, [reportRendered]);

  const transitionToRest = useCallback(() => {
    setAudioLevel(0);
    transition(restState(modeRef.current));
  }, [transition]);

  const applyRecordingStatus = useCallback((status: RecordingStatus) => {
    if (!shouldApplyRecordingStatus(latestRecordingRevisionRef.current, status)) return;
    latestRecordingRevisionRef.current = status.revision;
    activeSessionIdRef.current = status.session_id ?? null;

    if (status.phase === "starting") {
      setFailure(null);
      setRetrying(false);
      setProgressMessage(null);
      setAudioLevel(0);
      transition("appearing");
    } else if (status.phase === "recording") {
      setFailure(null);
      setAudioLevel(0);
      transition("recording");
    } else if (status.phase === "stopping") {
      setAudioLevel(0);
      transition("stopping");
    } else if (status.phase === "cancelling") {
      transitionToRest();
    } else if (!(["success", "error", "no_speech"] as GadgetState[]).includes(stateRef.current)) {
      transitionToRest();
    }
  }, [transition, transitionToRest]);

  useEffect(() => {
    let mounted = true;
    let subscriptions: Array<() => void> = [];

    const setup = async () => {
      subscriptions = await Promise.all([
        listen<RecordingStatus>("recording-initializing", (event) => applyRecordingStatus(event.payload)),
        listen<RecordingStatus>("recording-started", (event) => applyRecordingStatus(event.payload)),
        listen<RecordingStatus>("recording-stopped", (event) => applyRecordingStatus(event.payload)),
        listen<RecordingStatus>("recording-cancelled", (event) => applyRecordingStatus(event.payload)),
        listen<RecordingStatus>("recording-idle", (event) => applyRecordingStatus(event.payload)),
        listen<RecordingStatus>("recording-no-speech", ({ payload }) => {
          if (!payload.session_id || payload.session_id !== activeSessionIdRef.current
            || payload.revision < latestRecordingRevisionRef.current) return;
          setFailure(null);
          setRetrying(false);
          setProgressMessage(null);
          transition("no_speech");
        }),
        listen<number>("audio-level", (event) => {
        const level = Number.isFinite(event.payload) ? Math.max(0, Math.min(1, event.payload)) : 0;
        setAudioLevel((previous) => Math.abs(previous - level) < 0.005 ? previous : level);
        }),
        listen<{ active: boolean; operation_id: number; cancelled: boolean }>("transcribing", (event) => {
        if (event.payload.operation_id < operationRef.current) return;
        operationRef.current = event.payload.operation_id;
        if (event.payload.cancelled) { transitionToRest(); return; }
        if (event.payload.active) {
          transition("processing");
        } else if (["processing", "processing_long", "stopping"].includes(stateRef.current)) {
          setFailure({ id: "", message: "Não foi possível concluir o ditado.", canRetry: false });
          transition("error");
        }
        }),
        listen<number>("operation-cancelled", (event) => { if (event.payload < operationRef.current) return; operationRef.current = event.payload; setRetrying(false); transitionToRest(); }),
        listen<string>("capture-error", (event) => { setFailure({ id: "", message: event.payload, canRetry: false }); transition("error"); }),
        listen<string>("storage-error", (event) => { setFailure({ id: "", message: event.payload, canRetry: false }); transition("error"); }),
        listen<HistoryEntry>("transcription-saved", (event) => {
        const entry = event.payload;
        if (entry.source && entry.source !== "mic") return;
        if (!belongsToRecordingSession(entry, activeSessionIdRef.current)) return;
        setRetrying(false);
        setProgressMessage(null);
        if (entry.is_error) {
          setFailure({
            id: entry.id,
            message: entry.error_message || "Não foi possível transcrever.",
            canRetry: Boolean(entry.audio_path) && !entry.text,
          });
          transition("error");
        } else {
          setFailure(null);
          transition("success");
        }
        }),
        listen<PipelineProgressEvent>("pipeline-progress", (event) => {
        const progress = event.payload;
        if (progress.operation_id < operationRef.current) return;
        operationRef.current = progress.operation_id;
        if (progress.kind === "complete") {
          setProgressMessage(null);
          return;
        }
        if (progress.message) setProgressMessage(progress.message);
        if (progress.kind === "fallback_started") transition("processing_long");
        }),
        listen<WidgetPreferences>("widget-preferences-changed", (event) => {
        const mode = event.payload.visibility_mode;
        modeRef.current = mode;
        setVisibilityMode(mode);
        if (["hidden", "idle", "hover"].includes(stateRef.current)) transition(restState(mode));
        }),
      ]);

      if (!mounted) {
        subscriptions.forEach((unlisten) => unlisten());
        subscriptions = [];
        return;
      }

      const [status, preferences, shortcuts] = await Promise.all([
        getRecordingStatus(),
        getWidgetPreferences(),
        getShortcuts(),
      ]);
      if (!mounted) return;
      modeRef.current = preferences.visibility_mode;
      setVisibilityMode(preferences.visibility_mode);
      setShortcut(shortcutLabel(shortcuts.toggle));
      applyRecordingStatus(status);
    };
    void setup().catch((error) => console.error("gadget bootstrap failed:", error));

    return () => {
      mounted = false;
      subscriptions.forEach((unlisten) => unlisten());
    };
  }, [applyRecordingStatus, transition]);

  useEffect(() => {
    const timeout = GADGET_STATES[state].timeoutMs;
    if (timeout === null) return;
    const timer = window.setTimeout(() => {
      const next = stateAfterTimeout(stateRef.current, modeRef.current);
      if (next) transition(next);
    }, timeout);
    return () => window.clearTimeout(timer);
  }, [state, transition]);

  useEffect(() => {
    reportRendered();
    const element = pillRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => reportRendered());
    observer.observe(element);
    return () => observer.disconnect();
  }, [state, reportRendered]);

  useEffect(() => {
    const handleNativeRepaint = () => {
      window.requestAnimationFrame(() => reportRendered(true));
    };
    window.addEventListener("sonora-gadget-repaint", handleNativeRepaint);
    return () => window.removeEventListener("sonora-gadget-repaint", handleNativeRepaint);
  }, [reportRendered]);

  const startOrStop = async () => {
    if (["idle", "hover"].includes(stateRef.current)) transition("appearing");
    if (stateRef.current === "recording") transition("stopping");
    try {
      await toggleRecordingState();
    } catch (error) {
      setFailure({ id: "", message: typeof error === "string" ? error : "Não foi possível acessar o microfone.", canRetry: false });
      transition("error");
    }
  };

  const cancel = async () => {
    try {
      await cancelRecording();
    } catch (error) {
      console.error("cancel_recording failed:", error);
    }
  };

  const retry = async () => {
    if (!failure?.canRetry || retrying) return;
    setRetrying(true);
    transition("processing_long");
    try {
      await retryTranscription(failure.id);
    } catch (error) {
      setRetrying(false);
      setFailure((current) => current ? { ...current, message: typeof error === "string" ? error : "A retranscrição falhou." } : current);
      transition("error");
    }
  };

  if (state === "hidden") return null;

  return (
    <div className="gadget-window">
      <div
        ref={pillRef}
        className={`gadget-stage gadget-stage--${GADGET_STATES[state].animation}`}
        onPointerEnter={() => {
          if (stateRef.current === "idle" && visibilityMode === "always") transition("hover");
        }}
        onPointerLeave={() => {
          if (stateRef.current === "hover") transition("idle");
        }}
        aria-live="polite"
        aria-label={GADGET_STATES[state].accessibleLabel}
      >
        <GadgetPill
          state={state}
          level={audioLevel}
          shortcut={shortcut}
          failure={failure}
          retrying={retrying}
          progressMessage={progressMessage}
          onToggle={() => void startOrStop()}
          onCancel={() => void cancel()}
          onRetry={() => void retry()}
        />
      </div>
    </div>
  );
}

interface GadgetPillProps {
  state: Exclude<GadgetState, "hidden">;
  level: number;
  shortcut: string;
  failure: { id: string; message: string; canRetry: boolean } | null;
  retrying: boolean;
  progressMessage: string | null;
  onToggle: () => void;
  onCancel: () => void;
  onRetry: () => void;
}

function GadgetPill({ state, level, shortcut, failure, retrying, progressMessage, onToggle, onCancel, onRetry }: GadgetPillProps) {
  if (state === "idle") {
    return <button type="button" onClick={onToggle} className="sonora-bar sonora-bar--idle" aria-label={`Iniciar ditado. ${shortcut}`}><MiniWave /></button>;
  }
  if (state === "hover") {
    return (
      <button type="button" onClick={onToggle} className="sonora-bar sonora-bar--hover" aria-label={`Iniciar ditado. ${shortcut}`}>
        <MiniWave /><Mic className="h-[15px] w-[15px]" aria-hidden /><span className="sonora-bar__shortcut">{shortcut}</span>
      </button>
    );
  }
  if (state === "recording") {
    return (
      <div className="sonora-bar sonora-bar--recording" role="status">
        <BarButton label="Cancelar e descartar" onClick={onCancel}><X className="h-[17px] w-[17px]" strokeWidth={2} /></BarButton>
        <Waveform level={level} />
        <BarButton label="Parar e transcrever" onClick={onToggle} primary><Square className="h-[11px] w-[11px] fill-current" strokeWidth={1.5} /></BarButton>
      </div>
    );
  }
  if (state === "processing" || state === "processing_long") {
    return (
      <div className={`sonora-bar ${state === "processing" ? "sonora-bar--processing" : "sonora-bar--processing-long"}`} role="status">
        <ProcessingDots /><BarButton label="Cancelar processamento" onClick={onCancel}><X className="h-4 w-4" /></BarButton>
        {showsProcessingLabel(state) && <span className="sonora-bar__status">{retrying ? "Tentando novamente…" : progressMessage || "Transcrevendo…"}</span>}
      </div>
    );
  }
  if (state === "success") {
    return <div className="sonora-bar sonora-bar--success" role="status" aria-label="Texto disponível"><Check className="h-[17px] w-[17px]" strokeWidth={2.2} aria-hidden /></div>;
  }
  if (state === "no_speech") {
    return <div className="sonora-bar sonora-bar--no-speech" role="status" aria-live="polite"><MicOff className="h-4 w-4 shrink-0" aria-hidden /><span>Nenhuma voz encontrada</span></div>;
  }
  if (state === "error") {
    return (
      <div className="sonora-bar sonora-bar--error" role="alert" title={failure?.message}>
        <AlertCircle className="h-4 w-4 shrink-0 text-[#ffaaa3]" aria-hidden />
        <span className="sonora-bar__error-text">{failure?.message || "Não foi possível concluir"}</span>
        {failure?.canRetry ? (
          <button type="button" onClick={onRetry} disabled={retrying} className="sonora-bar__retry" aria-label="Tentar transcrever o áudio salvo novamente">
            <RefreshCw className={`h-3.5 w-3.5 ${retrying ? "animate-spin" : ""}`} aria-hidden /><span>Tentar novamente</span>
          </button>
        ) : (
          <button type="button" onClick={onCancel} className="sonora-bar__dismiss" aria-label="Dispensar erro"><X className="h-3.5 w-3.5" aria-hidden /></button>
        )}
      </div>
    );
  }
  return <div className="sonora-bar sonora-bar--activity" role="status"><ActivityMark settled={state === "stopping"} /></div>;
}

function BarButton({ label, onClick, primary = false, children }: { label: string; onClick: () => void; primary?: boolean; children: React.ReactNode }) {
  return <button type="button" onClick={onClick} className={`sonora-bar__control ${primary ? "sonora-bar__control--primary" : ""}`} aria-label={label}>{children}</button>;
}

function ActivityMark({ settled = false }: { settled?: boolean }) {
  return <span className={`activity-mark ${settled ? "activity-mark--settled" : ""}`} aria-hidden><span /><span /><span /></span>;
}

function ProcessingDots() {
  return <span className="processing-dots" aria-hidden><span /><span /><span /></span>;
}

function MiniWave() {
  return <span className="mini-wave" aria-hidden>{[5, 10, 15, 9, 5].map((height, index) => <span key={index} style={{ height }} />)}</span>;
}

function Waveform({ level }: { level: number }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const targetRef = useRef(0);
  const envelopeRef = useRef(0);

  useEffect(() => {
    targetRef.current = Math.min(1, Math.max(0, level));
  }, [level]);

  useEffect(() => {
    let frame = 0;
    let phase = 0;
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const tick = () => {
      const target = targetRef.current;
      const current = envelopeRef.current;
      envelopeRef.current += (target - current) * (target > current ? 0.42 : 0.12);
      const envelope = envelopeRef.current < 0.008 ? 0 : envelopeRef.current;
      if (!reduceMotion && envelope > 0) phase += 0.11 + envelope * 0.035;
      Array.from(containerRef.current?.children ?? []).forEach((child, index) => {
        const weight = WAVE_WEIGHTS[index] ?? 0.3;
        const organic = envelope > 0 && !reduceMotion ? 0.92 + Math.sin(phase + index * 0.73) * 0.09 : 1;
        const height = 3 + envelope * 25 * weight * organic;
        const bar = child as HTMLElement;
        bar.style.height = `${Math.max(3, Math.min(28, height))}px`;
        bar.style.opacity = `${0.42 + envelope * 0.58}`;
      });
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, []);

  return <div ref={containerRef} className="live-wave" aria-hidden>{Array.from({ length: BAR_COUNT }).map((_, index) => <span key={index} />)}</div>;
}

/** Deterministic visual QA surface; reachable only from the Vite dev build. */
export function GadgetPreviewApp() {
  const [level, setLevel] = useState(0.66);
  const states: Array<Exclude<GadgetState, "hidden">> = ["idle", "hover", "initializing", "recording", "processing", "processing_long", "success", "no_speech", "error"];
  return (
    <main className="gadget-preview">
      <header><h1>Sonora Bar — visual QA</h1><label>RMS<input type="range" min="0" max="1" step="0.01" value={level} onChange={(event) => setLevel(Number(event.target.value))} /></label></header>
      <div className="gadget-preview__grid">
        {states.map((previewState) => (
          <figure key={previewState}>
            <div className="gadget-preview__stage">
              <GadgetPill state={previewState} level={level} shortcut="Ctrl + B" failure={{ id: "preview", message: "Falha na transcrição", canRetry: true }} retrying={false} progressMessage={previewState === "processing_long" ? "Modelo indisponível · usando Whisper" : null} onToggle={() => undefined} onCancel={() => undefined} onRetry={() => undefined} />
            </div>
            <figcaption>{previewState}</figcaption>
          </figure>
        ))}
      </div>
    </main>
  );
}
