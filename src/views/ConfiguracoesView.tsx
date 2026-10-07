import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Plus,
  X,
  Zap,
  Rocket,
  Target,
  Gem,
  KeyRound,
  Activity,
  Mic,
  Eye,
  EyeOff,
  Save,
  CheckCircle2,
  FolderOpen,
  HardDrive,
  RotateCcw,
  Settings2,
  SlidersHorizontal,
  BookOpen,
  BrainCircuit,
  ChevronRight,
  type LucideIcon,
} from "lucide-react";
import {
  getWidgetPreferences,
  setWidgetVisibilityMode,
  listAudioDevices,
  getInputDevice,
  setInputDevice,
  startMicTest,
  stopMicTest,
  onMicTestLevel,
  getEngineConfig,
  getVocabulary,
  setVocabulary,
  getDevMode,
  setDevMode,
  getSanitizerEnabled,
  setSanitizerEnabled,
  getAudioStorageConfig,
  setAudioStorageDirectory,
  getModeConfig,
  updateModeConfig,
  getApiKeys,
  saveApiKeys,
  type DeepgramMode,
  type SanitizerModel,
  type TranscriptionMode,
  type GeminiModel,
  type GeminiProvider,
  type GeminiPipelineChoice,
  type GeminiPipelineConfig,
  type OpenRouterWhisperModel,
  type VocabularyTerm,
  type VocabularyCategory,
  type WidgetVisibilityMode,
} from "../lib/tauri";
import { Button } from "../components/ui/Button";
import { Card } from "../components/ui/Card";
import { Input } from "../components/ui/Input";
import { Toggle } from "../components/ui/Toggle";
import { PageHeader, PreferenceRow } from "../components/ui/Surface";
import { IntelligenceSettings } from "./IntelligenceSettings";

type Tab =
  | "geral"
  | "pipelines"
  | "provedores"
  | "vocabulario"
  | "inteligencia"
  | "diagnostico";

const TABS: { key: Tab; label: string; description: string; Icon: LucideIcon }[] = [
  { key: "geral", label: "Geral", description: "Inicialização, áudio e armazenamento", Icon: Settings2 },
  { key: "pipelines", label: "Pipelines", description: "Velocidade, precisão e conteúdo", Icon: SlidersHorizontal },
  { key: "provedores", label: "Provedores e APIs", description: "Conexões e chaves locais", Icon: KeyRound },
  { key: "vocabulario", label: "Vocabulário", description: "Grafias e variações faladas", Icon: BookOpen },
  { key: "inteligencia", label: "Inteligência e privacidade", description: "Contexto, styles, snippets e destino", Icon: BrainCircuit },
  { key: "diagnostico", label: "Diagnóstico", description: "Logs e informações técnicas", Icon: Activity },
];

function displayWindowsPath(path: string): string {
  if (path.startsWith("\\\\?\\UNC\\")) return `\\\\${path.slice(8)}`;
  if (path.startsWith("\\\\?\\")) return path.slice(4);
  return path;
}

const MODE_CARDS: {
  id: TranscriptionMode;
  title: string;
  engine: string;
  blurb: string;
  badge?: string;
  Icon: LucideIcon;
}[] = [
  {
    id: "ultra-fast",
    title: "Ultrarrápido",
    engine: "OpenRouter STT · Groq",
    blurb: "Menor latência",
    Icon: Zap,
  },
  {
    id: "fast-accurate",
    title: "Rápido e preciso",
    engine: "Gemini com áudio",
    blurb: "Boa precisão com menos etapas",
    Icon: Rocket,
  },
  {
    id: "precise",
    title: "Preciso",
    engine: "Whisper + Gemini",
    blurb: "Melhor equilíbrio geral",
    Icon: Target,
  },
  {
    id: "ultra-precise",
    title: "Ultrapreciso",
    engine: "Whisper → Sanitizer → Gemini",
    blurb: "Para conteúdo importante",
    Icon: Gem,
  },
];

export function ConfiguracoesView() {
  const [tab, setTab] = useState<Tab>("geral");
  const currentTab = TABS.find((item) => item.key === tab)!;

  return (
    <div className="settings-page space-y-7">
      <PageHeader title="Configurações" description="Ajuste o Sonora para o seu fluxo de trabalho." />
      <div className="settings-layout">
      <nav
        className="settings-nav scrollbar-thin"
        aria-label="Seções de configuração"
      >
        {TABS.map((t) => (
          <button
            key={t.key}
            aria-current={tab === t.key ? "page" : undefined}
            onClick={() => setTab(t.key)}
            className={
              "settings-nav__item group gap-3 rounded-[9px] px-3 py-2.5 transition-colors " +
              (tab === t.key
                ? "bg-[#e9e9e4] text-ink"
                : "text-[#65665f] hover:bg-[#efefeb] hover:text-ink")
            }
          >
            <t.Icon className="h-4 w-4 shrink-0" aria-hidden />
            <span className="min-w-0 flex-1">
              <span className="block text-[13px] font-medium">{t.label}</span>
              <span className="settings-nav__description mt-0.5 block truncate text-[11px] text-muted">{t.description}</span>
            </span>
            <ChevronRight className="settings-nav__chevron h-3.5 w-3.5 text-[#a0a19a]" aria-hidden />
          </button>
        ))}
      </nav>
      <section className="settings-content" aria-labelledby={`settings-${tab}`}>
        <header className="mb-7 border-b border-line pb-5">
          <h2 id={`settings-${tab}`} className="text-[20px] font-semibold tracking-[-0.02em] text-ink">{currentTab.label}</h2>
          <p className="mt-1 text-[13px] text-muted">{currentTab.description}</p>
        </header>
        {tab === "geral" && <GeralTab />}
        {tab === "pipelines" && <PipelinesTab />}
        {tab === "provedores" && <ProvedoresTab />}
        {tab === "vocabulario" && <VocabularioTab />}
        {tab === "inteligencia" && <IntelligenceSettings />}
        {tab === "diagnostico" && <DiagnosticoTab />}
      </section>
      </div>
    </div>
  );
}

/* --------------------------------- Geral --------------------------------- */

function GeralTab() {
  const [startup, setStartup] = useState(false);
  const [widgetVisibility, setWidgetVisibility] = useState<WidgetVisibilityMode>("auto");
  const [devices, setDevices] = useState<string[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null);
  const [isTesting, setIsTesting] = useState(false);
  const [micLevel, setMicLevel] = useState(0);
  const [audioDirectory, setAudioDirectory] = useState("");
  const [defaultAudioDirectory, setDefaultAudioDirectory] = useState("");
  const [customAudioDirectory, setCustomAudioDirectory] = useState(false);
  const [audioDirectoryStatus, setAudioDirectoryStatus] = useState("");

  useEffect(() => {
    invoke<boolean>("get_autostart")
      .then(setStartup)
      .catch((e) => console.error("get_autostart failed:", e));
    getWidgetPreferences()
      .then((preferences) => setWidgetVisibility(preferences.visibility_mode))
      .catch((e) => console.error("get_widget_preferences failed:", e));
    listAudioDevices()
      .then(setDevices)
      .catch((e) => console.error("listAudioDevices failed:", e));
    getInputDevice()
      .then(setSelectedDevice)
      .catch((e) => console.error("getInputDevice failed:", e));
    getAudioStorageConfig()
      .then((config) => {
        setAudioDirectory(displayWindowsPath(config.effective_directory));
        setDefaultAudioDirectory(displayWindowsPath(config.default_directory));
        setCustomAudioDirectory(Boolean(config.custom_directory));
      })
      .catch((e) => console.error("getAudioStorageConfig failed:", e));
  }, []);

  useEffect(() => {
    if (!isTesting) {
      setMicLevel(0);
      return;
    }
    let active = true;
    let unlisten: (() => void) | null = null;
    onMicTestLevel((level) => {
      if (active) setMicLevel(level);
    })
      .then((unsub) => {
        if (active) unlisten = unsub;
        else unsub();
      })
      .catch(() => {});
    return () => {
      active = false;
      unlisten?.();
    };
  }, [isTesting]);

  useEffect(() => {
    return () => {
      stopMicTest().catch(() => {});
    };
  }, []);

  return (
    <div className="space-y-8">
      <div className="divide-y divide-line">
        <Row
          title="Iniciar com o Windows"
          description="Abre o Sonora ao ligar o computador (em segundo plano se usar autostart)."
        >
          <Toggle
            label="Iniciar com o Windows"
            checked={startup}
            onChange={(v) => {
              setStartup(v);
              invoke("set_autostart", { enabled: v }).catch(console.error);
            }}
          />
        </Row>
        <Row
          title="Barra de ditado"
          description={widgetVisibility === "always" ? "Mantém uma pequena cápsula visível quando não há ditado ativo." : "Mostra a barra somente durante gravação, processamento e feedback."}
        >
          <Toggle
            label="Sempre mostrar a barra de ditado"
            checked={widgetVisibility === "always"}
            onChange={(v) => {
              const mode: WidgetVisibilityMode = v ? "always" : "auto";
              setWidgetVisibility(mode);
              setWidgetVisibilityMode(mode)
                .then((preferences) => setWidgetVisibility(preferences.visibility_mode))
                .catch(console.error);
            }}
          />
        </Row>
        <Row
          title="Bandeja do sistema"
          description="Ao fechar a janela, o app continua na bandeja. Use Sair no menu do ícone para encerrar de verdade."
        >
          <span className="shrink-0 text-[12px] font-medium text-[#25613f]">
            Sempre ativo
          </span>
        </Row>
      </div>

      <section className="border-t border-line pt-7">
        <div className="flex items-center gap-2.5">
          <Mic className="h-4 w-4 text-[#595a54]" />
          <label htmlFor="microphone-input" className="text-[14px] font-medium text-ink">
            Microfone de entrada
          </label>
        </div>
        <p className="mt-1.5 text-[13px] leading-5 text-muted">
          Escolha o microfone das gravações. Se o dispositivo sumir, o app usa o
          padrão do sistema.
        </p>
        <div className="mt-5 flex flex-col items-stretch gap-3 sm:flex-row sm:items-center">
          <select
            id="microphone-input"
            value={selectedDevice || "default"}
            onChange={async (e) => {
              const val = e.target.value === "default" ? null : e.target.value;
              setSelectedDevice(val);
              await setInputDevice(val);
              setDevices(await listAudioDevices());
            }}
            className="h-10 w-full max-w-md rounded-[9px] border border-line bg-white px-3 text-[13px] text-ink outline-hidden"
          >
            <option value="default">Padrão do sistema</option>
            {devices.map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
          <Button
            variant={isTesting ? "secondary" : "primary"}
            onClick={async () => {
              if (isTesting) {
                await stopMicTest();
                setIsTesting(false);
              } else {
                await startMicTest();
                setIsTesting(true);
              }
            }}
            className="gap-2"
          >
            <span
              className={
                "h-2 w-2 rounded-full " +
                (isTesting ? "animate-pulse bg-[#b8352d]" : "bg-[#8b8c85]")
              }
            />
            {isTesting ? "Parar teste" : "Testar microfone"}
          </Button>
        </div>
        {isTesting && (
          <div className="mt-4 space-y-2" aria-live="polite">
            <div className="flex justify-between text-[11px] text-muted">
              <span>Nível de entrada</span>
              <span className="font-mono">{Math.round(micLevel * 100)}%</span>
            </div>
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-[#e8e8e2]">
              <div
                className="h-full rounded-full bg-[#252522] transition-[width] duration-75"
                style={{ width: `${micLevel * 100}%` }}
              />
            </div>
          </div>
        )}
      </section>

      <section className="border-t border-line pt-7">
        <div className="flex items-center gap-2.5">
          <HardDrive className="h-4 w-4 text-[#595a54]" aria-hidden />
          <h3 className="text-[14px] font-medium text-ink">
            Pasta dos áudios transcritos
          </h3>
        </div>
        <p className="mt-1.5 max-w-[72ch] text-[13px] leading-5 text-muted">
          Define onde as próximas gravações e cópias de áudios enviados serão
          salvas. Arquivos existentes continuam no local atual e permanecem
          acessíveis pelo Histórico.
        </p>
        <div className="mt-5 flex flex-col gap-3 lg:flex-row lg:items-center">
          <Input
            name="audio-storage-directory"
            value={audioDirectory}
            readOnly
            className="min-w-0 flex-1 font-mono text-xs"
            aria-label="Pasta atual dos áudios transcritos"
          />
          <div className="flex flex-wrap gap-2">
            <Button
              variant="primary"
              className="gap-2 text-xs"
              onClick={async () => {
                setAudioDirectoryStatus("");
                const selected = await open({
                  directory: true,
                  multiple: false,
                  title: "Escolher pasta para os áudios transcritos",
                });
                if (typeof selected !== "string") return;
                try {
                  const config = await setAudioStorageDirectory(selected);
                  setAudioDirectory(displayWindowsPath(config.effective_directory));
                  setDefaultAudioDirectory(displayWindowsPath(config.default_directory));
                  setCustomAudioDirectory(Boolean(config.custom_directory));
                  setAudioDirectoryStatus("Pasta atualizada");
                } catch (error) {
                  setAudioDirectoryStatus(String(error));
                }
              }}
            >
              <FolderOpen className="h-4 w-4" aria-hidden />
              Escolher pasta
            </Button>
            {customAudioDirectory && (
              <Button
                variant="secondary"
                className="gap-2 text-xs"
                title={defaultAudioDirectory}
                onClick={async () => {
                  try {
                    const config = await setAudioStorageDirectory(null);
                    setAudioDirectory(displayWindowsPath(config.effective_directory));
                    setDefaultAudioDirectory(displayWindowsPath(config.default_directory));
                    setCustomAudioDirectory(false);
                    setAudioDirectoryStatus("Pasta padrão restaurada");
                  } catch (error) {
                    setAudioDirectoryStatus(String(error));
                  }
                }}
              >
                <RotateCcw className="h-4 w-4" aria-hidden />
                Usar padrão
              </Button>
            )}
          </div>
        </div>
        <div className="mt-3 flex flex-wrap items-center justify-between gap-2 text-[11px]">
          <span className="text-muted">
            {customAudioDirectory ? "Local personalizado" : "Local padrão do aplicativo"}
          </span>
          {audioDirectoryStatus && (
            <span className="text-[#25613f]" role="status">
              {audioDirectoryStatus}
            </span>
          )}
        </div>
      </section>
    </div>
  );
}

/* ------------------------------- Pipelines ------------------------------- */

function PipelinesTab() {
  const [mode, setMode] = useState<TranscriptionMode>("ultra-fast");
  const [fileTaggingEnabled, setFileTaggingEnabled] = useState(true);
  const [geminiFallback, setGeminiFallback] = useState(true);
  const [sanitizer, setSanitizer] = useState<SanitizerModel>("llama-70b");
  const [sanitizerEnabled, setSanitizerEnabledState] = useState(true);
  const [engine, setEngine] = useState("groq-whisper");
  const [dual, setDual] = useState(false);
  const [deepgramMode, setDeepgramMode] = useState<DeepgramMode>("batch");
  const [reasoning, setReasoning] = useState(false);
  const [effort, setEffort] = useState("medium");
  const [status, setStatus] = useState("");
  const [geminiPipelines, setGeminiPipelines] = useState<GeminiPipelineConfig>({
    ultra_fast_whisper: "large-v3-turbo",
    fast_accurate: {
      model: "flash-lite35",
      provider: "google-ai-studio",
      use_custom_model: false,
      custom_model: "",
    },
    precise: {
      model: "flash-lite35",
      provider: "google-ai-studio",
      use_custom_model: false,
      custom_model: "",
    },
    ultra_precise: {
      model: "flash-lite35",
      provider: "google-ai-studio",
      use_custom_model: false,
      custom_model: "",
    },
  });

  const persistMode = (p: {
    mode?: TranscriptionMode;
    gemini_fallback_to_whisper?: boolean;
    file_tagging_enabled?: boolean;
    gemini_pipelines?: GeminiPipelineConfig;
  }) => {
    const payload = {
      modes_enabled: true,
      mode: p.mode ?? mode,
      gemini_fallback_to_whisper: p.gemini_fallback_to_whisper ?? geminiFallback,
      file_tagging_enabled: p.file_tagging_enabled ?? fileTaggingEnabled,
      gemini_pipelines: p.gemini_pipelines ?? geminiPipelines,
    };
    updateModeConfig(payload)
      .then(() => setStatus("Pipeline salva"))
      .catch((e) => setStatus(String(e)));
  };

  useEffect(() => {
    getModeConfig()
      .then((m) => {
        setMode(m.mode);
        setGeminiFallback(m.gemini_fallback_to_whisper);
        setFileTaggingEnabled(m.file_tagging_enabled);
        setGeminiPipelines(m.gemini_pipelines);
      })
      .catch(console.error);
    getEngineConfig()
      .then((c) => {
        if (c.engine) setEngine(c.engine);
        setDual(!!c.dual_engine);
        if (c.sanitizer) setSanitizer(c.sanitizer as SanitizerModel);
        setReasoning(!!c.reasoning_enabled);
        if (c.reasoning_effort) setEffort(c.reasoning_effort);
        if (c.deepgram_mode) setDeepgramMode(c.deepgram_mode);
      })
      .catch(console.error);
    getSanitizerEnabled().then(setSanitizerEnabledState).catch(console.error);
  }, []);

  const selected = MODE_CARDS.find((c) => c.id === mode)!;
  const SelectedIcon = selected.Icon;
  const selectedRouteKey =
    mode === "fast-accurate"
      ? "fast_accurate"
      : mode === "precise"
        ? "precise"
        : mode === "ultra-precise"
          ? "ultra_precise"
          : null;

  const selectMode = (id: TranscriptionMode) => {
    setMode(id);
    persistMode({ mode: id });
  };

  const updateGeminiRoute = (
    key: "fast_accurate" | "precise" | "ultra_precise",
    patch: Partial<GeminiPipelineChoice>,
    shouldPersist = true,
  ) => {
    const next = {
      ...geminiPipelines,
      [key]: { ...geminiPipelines[key], ...patch },
    };
    setGeminiPipelines(next);
    if (shouldPersist) persistMode({ gemini_pipelines: next });
  };

  return (
    <div className="space-y-8">
      <div>
        <h3 className="mb-3 text-[14px] font-medium text-ink">
          Escolha um modo
        </h3>
        <div className="pipeline-workbench">
          <div className="pipeline-mode-list" role="group" aria-label="Modo de transcrição">
            {MODE_CARDS.map((card) => {
              const active = mode === card.id;
              const Icon = card.Icon;
              return (
                <button
                  key={card.id}
                  type="button"
                  aria-pressed={active}
                  onClick={() => selectMode(card.id)}
                  className={
                    "w-full rounded-[10px] border px-3 py-3 text-left transition-colors duration-150 " +
                    (active
                      ? "border-[#5c5d57] bg-[#f0f0eb]"
                      : "border-line bg-white hover:border-line-strong hover:bg-[#fafaf7]")
                  }
                >
                  <span className="flex items-start gap-3">
                    <span
                      className={
                        "flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] " +
                        (active
                          ? "bg-[#22221f] text-white"
                          : "bg-[#eeeeea] text-[#65665f]")
                      }
                    >
                      <Icon className="h-[18px] w-[18px]" aria-hidden />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="flex items-center justify-between gap-2">
                        <span className="text-[13px] font-medium text-ink">{card.title}</span>
                        {active && <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-[#4f504b]" aria-hidden />}
                      </span>
                      <span className="mt-0.5 block text-[11px] font-medium leading-4 text-[#555650]">{card.engine}</span>
                      <span className="mt-0.5 block text-[11px] leading-4 text-muted">{card.blurb}</span>
                      {card.badge && (
                        <span className="mt-1.5 inline-flex rounded-[6px] bg-[#f5ecd9] px-2 py-0.5 text-[10px] font-medium text-[#80551a]">
                          {card.badge}
                        </span>
                      )}
                    </span>
                  </span>
                </button>
              );
            })}
          </div>

          <section className="min-w-0 rounded-[12px] border border-line bg-[#f4f4ef]" aria-labelledby="active-pipeline-title">
            <header className="flex flex-wrap items-start justify-between gap-4 px-5 py-4">
              <div className="flex min-w-0 items-center gap-3">
                <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] border border-line bg-white text-[#4f504b]">
                  <SelectedIcon className="h-[18px] w-[18px]" aria-hidden />
                </span>
                <div className="min-w-0">
                  <h3 id="active-pipeline-title" className="text-[17px] font-semibold tracking-[-0.015em] text-ink">
                    Configurar {selected.title}
                  </h3>
                  <p className="mt-0.5 text-[12px] leading-5 text-muted">
                    {selected.engine} · {selected.blurb}
                  </p>
                </div>
              </div>
              {status && (
                <span className="inline-flex items-center gap-1.5 pt-1 text-xs text-[#25613f]" role="status">
                  <CheckCircle2 className="h-3.5 w-3.5" aria-hidden />
                  {status}
                </span>
              )}
            </header>

            {selectedRouteKey ? (
              <div className="grid gap-4 rounded-b-[11px] border-t border-line bg-white px-5 py-5 sm:grid-cols-2">
                    <label className="space-y-1.5 text-[12px] text-[#555650]">
                      <span>Modelo</span>
                      <select
                        name={`${selectedRouteKey}-gemini-model`}
                        value={geminiPipelines[selectedRouteKey].use_custom_model ? "custom" : geminiPipelines[selectedRouteKey].model}
                        onChange={(e) => {
                          if (e.target.value === "custom") {
                            updateGeminiRoute(
                              selectedRouteKey,
                              { use_custom_model: true },
                              Boolean(geminiPipelines[selectedRouteKey].custom_model.trim()),
                            );
                          } else {
                            updateGeminiRoute(selectedRouteKey, {
                              model: e.target.value as GeminiModel,
                              use_custom_model: false,
                            });
                          }
                        }}
                        className="h-10 w-full rounded-[9px] border border-line bg-white px-3 text-[12px] text-ink outline-hidden"
                      >
                        {geminiPipelines[selectedRouteKey].provider === "meta" ? (
                          <option value="muse-voice-transcribe-1.0">Muse Voice Transcribe (1.0)</option>
                        ) : (
                          <>
                            <option value="flash-lite35">Gemini 3.5 Flash-Lite</option>
                            <option value="flash36">Gemini 3.6 Flash</option>
                            {selectedRouteKey === "fast_accurate" && (
                              <option value="transcribe35">Gemini 3.5 Transcribe (Dedicado STT)</option>
                            )}
                          </>
                        )}
                        <option value="custom">ID customizado…</option>
                      </select>
                    </label>
                    <label className="space-y-1.5 text-[12px] text-[#555650]">
                      <span>Provedor</span>
                      <select
                        name={`${selectedRouteKey}-gemini-provider`}
                        value={geminiPipelines[selectedRouteKey].provider}
                        onChange={(e) => {
                          const provider = e.target.value as GeminiProvider;
                          if (provider === "meta") {
                            updateGeminiRoute(selectedRouteKey, {
                              provider,
                              model: "muse-voice-transcribe-1.0",
                              use_custom_model: false,
                            });
                          } else {
                            updateGeminiRoute(selectedRouteKey, {
                              provider,
                              model: geminiPipelines[selectedRouteKey].model === "muse-voice-transcribe-1.0"
                                ? "flash-lite35"
                                : geminiPipelines[selectedRouteKey].model,
                            });
                          }
                        }}
                        className="h-10 w-full rounded-[9px] border border-line bg-white px-3 text-[12px] text-ink outline-hidden"
                      >
                        <option value="google-ai-studio">Google AI Studio</option>
                        <option value="open-router">OpenRouter</option>
                        {selectedRouteKey === "fast_accurate" && (
                          <option value="meta">Meta</option>
                        )}
                      </select>
                    </label>
                    {geminiPipelines[selectedRouteKey].use_custom_model && (
                      <label className="space-y-1.5 text-[12px] text-[#555650] sm:col-span-2">
                        <span>ID do modelo customizado</span>
                        <Input
                          name={`${selectedRouteKey}-custom-model`}
                          value={geminiPipelines[selectedRouteKey].custom_model}
                          placeholder={
                            geminiPipelines[selectedRouteKey].provider === "meta"
                              ? "ex.: muse-voice-transcribe-1.0"
                              : geminiPipelines[selectedRouteKey].provider === "open-router"
                              ? "ex.: google/chirp-3 ou google/gemini-3.7-flash"
                              : "ex.: gemini-3.7-flash"
                          }
                          onChange={(e) =>
                            updateGeminiRoute(
                              selectedRouteKey,
                              { custom_model: e.target.value },
                              false,
                            )
                          }
                          onBlur={(e) => {
                            if (e.currentTarget.value.trim()) {
                              updateGeminiRoute(selectedRouteKey, {
                                custom_model: e.currentTarget.value.trim(),
                                use_custom_model: true,
                              });
                            }
                          }}
                        />
                      </label>
                    )}
                    {geminiPipelines[selectedRouteKey].provider === "meta" && (
                      <div className="space-y-2 sm:col-span-2">
                        <span className="text-[12px] font-medium text-[#555650]">
                          Idiomas da transcrição (Language Biasing & Code-Switching)
                        </span>
                        <div className="flex flex-wrap gap-2">
                          {[
                            { id: "Portuguese", label: "Português" },
                            { id: "English", label: "Inglês" },
                            { id: "Spanish", label: "Espanhol" },
                            { id: "French", label: "Francês" },
                            { id: "Italian", label: "Italiano" },
                            { id: "German", label: "Alemão" },
                            { id: "Japanese", label: "Japonês" },
                          ].map((lang) => {
                            const currentLangs =
                              geminiPipelines[selectedRouteKey].meta_languages &&
                              geminiPipelines[selectedRouteKey].meta_languages!.length > 0
                                ? geminiPipelines[selectedRouteKey].meta_languages!
                                : ["Portuguese", "English"];
                            const isSelected = currentLangs.includes(lang.id);
                            return (
                              <button
                                key={lang.id}
                                type="button"
                                aria-pressed={isSelected}
                                onClick={() => {
                                  let nextLangs: string[];
                                  if (isSelected) {
                                    if (currentLangs.length === 1) return;
                                    nextLangs = currentLangs.filter((l) => l !== lang.id);
                                  } else {
                                    nextLangs = [...currentLangs, lang.id];
                                  }
                                  updateGeminiRoute(selectedRouteKey, { meta_languages: nextLangs });
                                }}
                                className={
                                  "inline-flex min-h-8 items-center gap-1.5 rounded-[7px] border px-2.5 py-1 text-[11px] font-medium transition-colors " +
                                  (isSelected
                                    ? "border-[#5c5d57] bg-[#e8e8e3] text-ink font-semibold"
                                    : "border-line bg-white text-[#666] hover:bg-[#f4f4f0]")
                                }
                              >
                                {isSelected ? (
                                  <CheckCircle2 className="h-3 w-3" aria-hidden />
                                ) : (
                                  <Plus className="h-3 w-3" aria-hidden />
                                )}
                                {lang.label}
                              </button>
                            );
                          })}
                        </div>
                        <p className="text-[11px] leading-4 text-muted">
                          Focar em Português e Inglês evita que o modelo confunda fonemas e alucine palavras em Francês ou Espanhol, permitindo alternar livremente entre os dois idiomas na mesma fala.
                        </p>
                      </div>
                    )}
                    {geminiPipelines[selectedRouteKey].provider === "meta" ? (
                      <p className="text-[11px] leading-5 text-muted sm:col-span-2">
                        Meta Model API usa o modelo Muse Voice Transcribe 1.0 (ASR) com alta velocidade, reconhecimento nativo multilingue e vocabulário personalizado.
                      </p>
                    ) : geminiPipelines[selectedRouteKey].provider === "open-router" ? (
                      <p className="text-[11px] leading-5 text-muted sm:col-span-2">
                        A rota é automática: modelos dedicados como Chirp, Whisper e Transcribe usam Speech-to-Text; modelos com áudio usam Chat Completions.
                      </p>
                    ) : (
                      <p className="text-[11px] leading-5 text-muted sm:col-span-2">
                        O Google AI Studio usa modelos multimodais com áudio via Gemini API. Modelos STT dedicados do Google Cloud exigem outra API e outra credencial.
                      </p>
                    )}
              </div>
            ) : (
              <div className="rounded-b-[11px] border-t border-line bg-white px-5 py-5">
                <label className="block max-w-2xl space-y-1.5 text-[12px] text-[#555650]">
                      <span>Modelo Whisper via OpenRouter</span>
                      <select
                        name="ultra-fast-whisper-model"
                        value={geminiPipelines.ultra_fast_whisper}
                        onChange={(e) => {
                          const next = {
                            ...geminiPipelines,
                            ultra_fast_whisper: e.target.value as OpenRouterWhisperModel,
                          };
                          setGeminiPipelines(next);
                          persistMode({ gemini_pipelines: next });
                        }}
                    className="h-10 w-full rounded-[9px] border border-line bg-white px-3 text-[12px] text-ink outline-hidden"
                      >
                        <option value="large-v3-turbo">openai/whisper-large-v3-turbo</option>
                        <option value="large-v3">openai/whisper-large-v3</option>
                      </select>
                    </label>
                <p className="mt-2 max-w-[72ch] text-[11px] leading-5 text-muted">
                  Usa somente o endpoint de transcrição do OpenRouter, com o provedor Groq fixo e sem fallback para outro provedor.
                </p>
              </div>
            )}
          </section>
        </div>
      </div>

      <section className="border-t border-line pt-6">
        <h3 className="text-[14px] font-medium text-ink">Vibe coding</h3>
        <div className="surface-subtle mt-3 flex items-center justify-between gap-5 px-5 py-4">
          <div>
            <h4 className="text-[13px] font-medium text-ink">FileTagging</h4>
            <p className="mt-1 max-w-2xl text-[12px] leading-5 text-muted">
              Converte referências claras a arquivos em menções como @index.tsx.
              O texto é preparado para chats de código; o IDE decide como usar a menção.
            </p>
          </div>
          <Toggle
            label="Ativar FileTagging"
            checked={fileTaggingEnabled}
            onChange={(value) => {
              setFileTaggingEnabled(value);
              persistMode({ file_tagging_enabled: value });
            }}
          />
        </div>
      </section>

      {mode === "fast-accurate" && (
        <div className="surface-subtle flex items-center justify-between gap-4 px-5 py-4">
          <div>
            <h4 className="text-[13px] font-medium text-ink">
              Se o Gemini falhar, usar Whisper
            </h4>
            <p className="mt-1 text-[12px] text-muted">
              O histórico marca quando o fallback acontecer.
            </p>
          </div>
          <Toggle
            label="Usar Whisper se o Gemini falhar"
            checked={geminiFallback}
            onChange={(v) => {
              setGeminiFallback(v);
              persistMode({ gemini_fallback_to_whisper: v });
            }}
          />
        </div>
      )}

      {mode === "ultra-precise" && (
        <div className="surface-subtle space-y-3 p-5">
          <div className="flex items-center justify-between">
            <div>
              <h4 className="text-[13px] font-medium text-ink">
                Validador semântico
              </h4>
              <p className="mt-1 text-[12px] text-muted">
                Limpa ortografia após o Whisper no pipeline Ultrapreciso.
              </p>
            </div>
            <Toggle
              label="Ativar validador semântico"
              checked={sanitizerEnabled}
              onChange={(v) => {
                setSanitizerEnabledState(v);
                setSanitizerEnabled(v).catch(console.error);
              }}
            />
          </div>
          {sanitizerEnabled && (
            <div className="grid grid-cols-2 gap-2 pt-2">
              {(
                [
                  ["llama-70b", "LLaMA 70B"],
                  ["gpt-oss-20b", "GPT-OSS 20B"],
                  ["gpt-oss-120b", "GPT-OSS 120B"],
                  ["qwen3-27b", "Qwen 3.6 27B"],
                ] as const
              ).map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  onClick={() => {
                    setSanitizer(id);
                    invoke("update_engine_config", {
                      payload: {
                        engine,
                        sanitizer: id,
                        dual_engine: dual,
                        reasoning_enabled: reasoning,
                        reasoning_effort: effort,
                        deepgram_mode: deepgramMode,
                      },
                    }).catch(console.error);
                  }}
                  className={
                    "rounded-[8px] border py-2 text-[12px] font-medium " +
                    (sanitizer === id
                      ? "border-[#5c5d57] bg-[#e8e8e3] text-ink"
                      : "border-line bg-white text-[#555650] hover:bg-[#f4f4f0]")
                  }
                >
                  {label}
                </button>
              ))}
            </div>
          )}
        </div>
      )}

    </div>
  );
}

/* --------------------------- Provedores e APIs --------------------------- */

type KeyId = "groq" | "google" | "deepgram" | "openrouter" | "meta";

function ProvedoresTab() {
  const [keys, setKeys] = useState({
    groq: [] as string[],
    google: [] as string[],
    deepgram: [] as string[],
    openrouter: [] as string[],
    meta: [] as string[],
  });
  const [visible, setVisible] = useState<Record<string, boolean>>({});
  const [saving, setSaving] = useState<KeyId | null>(null);
  const [saved, setSaved] = useState<KeyId | null>(null);
  const [error, setError] = useState("");
  const [managing, setManaging] = useState<KeyId | null>(null);

  useEffect(() => {
    getApiKeys()
      .then((k) =>
        setKeys({
          groq: k.groq ?? [],
          google: k.google ?? [],
          deepgram: k.deepgram ?? [],
          openrouter: k.openrouter ?? [],
          meta: k.meta ?? [],
        }),
      )
      .catch(console.error);
  }, []);

  const providers: {
    id: KeyId;
    name: string;
    placeholder: string;
    help: string;
    requiredFor: string;
  }[] = [
    {
      id: "groq",
      name: "Groq",
      placeholder: "gsk_…",
      help: "Whisper, validador e fallbacks.",
      requiredFor: "Preciso, Ultrapreciso e fallbacks",
    },
    {
      id: "google",
      name: "Google (Gemini)",
      placeholder: "AIza…",
      help: "Transcrição Gemini e avaliação de pronúncia no Histórico.",
      requiredFor: "Rápido e preciso, Preciso, Ultrapreciso, Pronúncia",
    },
    {
      id: "openrouter",
      name: "OpenRouter",
      placeholder: "sk-or-v1-…",
      help: "Executa o Whisper do Ultrarrápido e os modelos customizados selecionados nos pipelines.",
      requiredFor: "Ultrarrápido e rotas OpenRouter dos demais pipelines",
    },
    {
      id: "meta",
      name: "Meta (Model API)",
      placeholder: "Chave de API da Meta (dev.meta.ai)",
      help: "Modelo Muse Voice Transcribe 1.0 (ASR) no pipeline Rápido e preciso.",
      requiredFor: "Rápido e preciso (com provedor Meta)",
    },
  ];

  const statusFor = (id: KeyId) => {
    const count = keys[id].filter((key) => key.trim()).length;
    return { label: count ? `${count} ${count === 1 ? "chave" : "chaves"}` : "Sem chaves" };
  };

  const save = async (id: KeyId) => {
    setSaving(id);
    setError("");
    try {
      const current = await getApiKeys();
      await saveApiKeys({ ...current, [id]: keys[id] });
      const stored = await getApiKeys();
      setKeys((draft) => ({ ...draft, [id]: stored[id] }));
      setVisible({});
      setSaved(id);
      window.setTimeout(() => setSaved((c) => (c === id ? null : c)), 2000);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setSaving(null);
    }
  };

  return (
    <div className="space-y-6">
      <p className="max-w-[72ch] text-[13px] leading-5 text-muted">
        As chaves são protegidas pela sua conta Windows. Credenciais salvas podem ser substituídas ou removidas; a interface não recupera seu valor.
      </p>
      {error && <div className="rounded-[10px] bg-[#fff1ef] px-4 py-3 text-[13px] text-[#9f2720]" role="alert">{error}</div>}
      <div className="divide-y divide-line border-y border-line">
        {providers.map((provider) => {
          const providerStatus = statusFor(provider.id);
          const isManaging = managing === provider.id;
          return (
            <section key={provider.id} className="py-1">
              <div className="grid min-h-[92px] grid-cols-[44px_minmax(0,1fr)_auto_auto] items-center gap-4 px-2 py-4 max-[820px]:grid-cols-[40px_minmax(0,1fr)_auto]">
                <span className="flex h-9 w-9 items-center justify-center rounded-full border border-line bg-white text-[#555650]">
                  <KeyRound className="h-4 w-4" aria-hidden />
                </span>
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <h3 className="text-[14px] font-medium text-ink">{provider.name}</h3>
                    <span className={"text-[11px] font-medium " + (keys[provider.id].some((key) => key.trim()) ? "text-[#25613f]" : "text-muted")}>
                      {keys[provider.id].some((key) => key.trim()) ? "Configurado · não verificado" : "Não configurado"}
                    </span>
                  </div>
                  <p className="mt-1 truncate text-[12px] text-muted" title={provider.requiredFor}>{provider.help}</p>
                </div>
                <span className="text-[12px] text-muted max-[820px]:hidden">{providerStatus.label}</span>
                <Button size="sm" onClick={() => setManaging(isManaging ? null : provider.id)} aria-expanded={isManaging}>
                  {isManaging ? "Fechar" : "Gerenciar"}<ChevronRight className={"h-3.5 w-3.5 transition-transform " + (isManaging ? "rotate-90" : "")} aria-hidden />
                </Button>
              </div>
              {isManaging && (
                <div className="mx-2 mb-5 rounded-[10px] bg-[#f4f4ef] p-4">
                  <p className="mb-4 text-[12px] leading-5 text-muted">{provider.requiredFor}</p>
                  <div className="space-y-2">
                    {keys[provider.id].map((key, index) => {
                      const visibilityKey = `${provider.id}-${index}`;
                      return (
                        <div key={visibilityKey} className="flex gap-2">
                          <div className="relative flex-1">
                            <Input
                              name={`${provider.id}-api-key-${index + 1}`}
                              type={visible[visibilityKey] ? "text" : "password"}
                              placeholder={key.startsWith("stored:") ? "Chave protegida · digite para substituir" : provider.placeholder}
                              value={key.startsWith("stored:") ? "" : key}
                              onChange={(event) => setKeys((current) => ({ ...current, [provider.id]: current[provider.id].map((item, itemIndex) => itemIndex === index ? event.target.value : item) }))}
                              autoComplete="off"
                              spellCheck={false}
                              className="pr-10 font-mono text-xs"
                              aria-label={`Chave ${index + 1} de ${provider.name}`}
                            />
                            <button type="button" className="icon-button absolute right-1 top-1" onClick={() => setVisible((current) => ({ ...current, [visibilityKey]: !current[visibilityKey] }))} aria-label={visible[visibilityKey] ? "Ocultar chave" : "Mostrar chave"}>
                              {visible[visibilityKey] ? <EyeOff className="h-4 w-4" aria-hidden /> : <Eye className="h-4 w-4" aria-hidden />}
                            </button>
                          </div>
                          <button type="button" onClick={() => setKeys((current) => ({ ...current, [provider.id]: current[provider.id].filter((_, itemIndex) => itemIndex !== index) }))} className="icon-button text-[#a72a21]" aria-label={`Remover chave ${index + 1} de ${provider.name}`}>
                            <X className="h-4 w-4" aria-hidden />
                          </button>
                        </div>
                      );
                    })}
                  </div>
                  <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
                    <Button size="sm" variant="ghost" onClick={() => setKeys((current) => ({ ...current, [provider.id]: [...current[provider.id], ""] }))}>
                      <Plus className="h-3.5 w-3.5" aria-hidden />Adicionar chave
                    </Button>
                    <Button size="sm" variant="primary" disabled={saving !== null} onClick={() => save(provider.id)}>
                      <Save className="h-3.5 w-3.5" aria-hidden />
                      {saving === provider.id ? "Salvando…" : saved === provider.id ? "Salvo" : "Salvar"}
                    </Button>
                  </div>
                </div>
              )}
            </section>
          );
        })}
      </div>
    </div>
  );
}

/* ------------------------------- Vocabulário ------------------------------ */

const VOCAB_CATEGORIES: { id: VocabularyCategory; label: string }[] = [
  { id: "ai_model", label: "Modelo de IA" },
  { id: "provider", label: "Provedor" },
  { id: "application", label: "Aplicativo" },
  { id: "person", label: "Pessoa" },
  { id: "file", label: "Arquivo" },
  { id: "command", label: "Comando" },
  { id: "function", label: "Função" },
  { id: "identifier", label: "Identificador" },
  { id: "study_term", label: "Termo de estudo" },
  { id: "other", label: "Outro" },
];

function emptyTerm(): VocabularyTerm {
  return {
    canonical: "",
    aliases: [],
    category: "other",
    strict: false,
    enabled: true,
  };
}

function VocabularioTab() {
  const [terms, setTerms] = useState<VocabularyTerm[]>([]);
  const [query, setQuery] = useState("");
  const [draft, setDraft] = useState<VocabularyTerm>(emptyTerm());
  const [aliasDraft, setAliasDraft] = useState("");
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    getVocabulary()
      .then(setTerms)
      .catch(console.error)
      .finally(() => setLoading(false));
  }, []);

  const persist = async (next: VocabularyTerm[]) => {
    setSaving(true);
    setError("");
    try {
      const cleaned = await setVocabulary(next);
      setTerms(cleaned);
      setDraft(emptyTerm());
      setAliasDraft("");
      setEditingIndex(null);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setSaving(false);
    }
  };

  const filtered = query.trim()
    ? terms.filter((t) => {
        const q = query.trim().toLowerCase();
        return (
          t.canonical.toLowerCase().includes(q) ||
          t.aliases.some((a) => a.toLowerCase().includes(q)) ||
          t.category.includes(q)
        );
      })
    : terms;

  const categoryLabel = (id: VocabularyCategory) =>
    VOCAB_CATEGORIES.find((c) => c.id === id)?.label ?? id;

  return (
    <div className="space-y-6">
      <div className="surface-subtle px-5 py-4">
        <h3 className="text-[14px] font-medium text-ink">
          Vocabulário estruturado
        </h3>
        <p className="mt-1.5 text-[12px] leading-5 text-muted">
          Cadastre a grafia correta e as variações da fala.{" "}
          <strong className="font-medium text-[#444540]">Literal</strong> protege arquivos,
          comandos e identificadores. A correção só age quando o encaixe é
          claro — sem substituição cega.
        </p>
      </div>

      <section className="surface space-y-4 p-5">
        <div className="flex items-center justify-between">
          <h4 className="text-[14px] font-medium text-ink">
            {editingIndex !== null ? "Editar termo" : "Novo termo"}
          </h4>
          {editingIndex !== null && (
            <button
              type="button"
              className="text-[12px] text-muted hover:text-ink"
              onClick={() => {
                setEditingIndex(null);
                setDraft(emptyTerm());
                setError("");
              }}
            >
              Cancelar
            </button>
          )}
        </div>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
          <div className="space-y-1.5 sm:col-span-2">
            <label htmlFor="vocabulary-canonical" className="field-label">
              Grafia correta
            </label>
            <Input
              id="vocabulary-canonical"
              placeholder="ex: provider-routing.json"
              value={draft.canonical}
              onChange={(e) =>
                setDraft((d) => ({ ...d, canonical: e.target.value }))
              }
              autoComplete="off"
              spellCheck={false}
            />
          </div>
          <div className="space-y-1.5">
            <label htmlFor="vocabulary-category" className="field-label">
              Categoria
            </label>
            <select
              id="vocabulary-category"
              value={draft.category}
              onChange={(e) =>
                setDraft((d) => ({
                  ...d,
                  category: e.target.value as VocabularyCategory,
                }))
              }
              className="h-10 w-full rounded-[9px] border border-line bg-white px-3 text-[13px] text-ink outline-hidden"
            >
              {VOCAB_CATEGORIES.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.label}
                </option>
              ))}
            </select>
          </div>
          <div className="flex items-end gap-4 pb-1">
            <label className="flex cursor-pointer items-center gap-2 text-[12px] text-[#555650]">
              <input
                type="checkbox" className="accent-[#1d1d1b]"
                checked={draft.strict}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, strict: e.target.checked }))
                }
              />
              Literal
            </label>
            <label className="flex cursor-pointer items-center gap-2 text-[12px] text-[#555650]">
              <input
                type="checkbox" className="accent-[#1d1d1b]"
                checked={draft.enabled}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, enabled: e.target.checked }))
                }
              />
              Ativo
            </label>
          </div>
        </div>
        <div className="space-y-1.5">
          <label htmlFor="vocabulary-alias" className="field-label">
            Variações da fala
          </label>
          <div className="flex gap-2">
            <Input
              id="vocabulary-alias"
              placeholder="ex: provider routing json"
              value={aliasDraft}
              onChange={(e) => setAliasDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  const a = aliasDraft.trim();
                  if (!a) return;
                  setDraft((d) => ({
                    ...d,
                    aliases: d.aliases.includes(a)
                      ? d.aliases
                      : [...d.aliases, a],
                  }));
                  setAliasDraft("");
                }
              }}
              className="flex-1"
              autoComplete="off"
              spellCheck={false}
            />
            <Button
              variant="secondary"
              type="button"
              onClick={() => {
                const a = aliasDraft.trim();
                if (!a) return;
                setDraft((d) => ({
                  ...d,
                  aliases: d.aliases.includes(a) ? d.aliases : [...d.aliases, a],
                }));
                setAliasDraft("");
              }}
            >
              Incluir
            </Button>
          </div>
          {draft.aliases.length > 0 && (
            <div className="flex flex-wrap gap-2 pt-1">
              {draft.aliases.map((a) => (
                <span
                  key={a}
                  className="inline-flex items-center gap-1.5 rounded-[7px] bg-[#ecece7] px-2.5 py-1 text-[11px] text-[#4f504b]"
                >
                  {a}
                  <button
                    type="button"
                    className="text-muted hover:text-[#a72a21]"
                    onClick={() =>
                      setDraft((d) => ({
                        ...d,
                        aliases: d.aliases.filter((x) => x !== a),
                      }))
                    }
                    aria-label={`Remover ${a}`}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </span>
              ))}
            </div>
          )}
        </div>
        {error && (
          <p className="text-[12px] text-[#a72a21]" role="alert">
            {error}
          </p>
        )}
        <Button
          variant="primary"
          className="gap-2"
          disabled={saving || !draft.canonical.trim()}
          onClick={() => {
            const canonical = draft.canonical.trim();
            if (!canonical) {
              setError("Informe a grafia correta.");
              return;
            }
            const term = { ...draft, canonical };
            if (editingIndex !== null) {
              const next = [...terms];
              next[editingIndex] = term;
              void persist(next);
            } else {
              void persist([...terms, term]);
            }
          }}
        >
          <Plus className="h-4 w-4" />
          {editingIndex !== null ? "Salvar" : "Adicionar termo"}
        </Button>
      </section>

      <Input
        aria-label="Buscar no vocabulário"
        placeholder="Buscar por grafia, variação ou categoria…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />

      {loading ? (
        <Card className="p-10 text-center text-[13px] text-muted">
          Carregando…
        </Card>
      ) : filtered.length === 0 ? (
        <Card className="p-12 text-center text-[13px] text-muted">
          {query.trim()
            ? "Nenhum termo na busca."
            : "Nenhum termo ainda. Cadastre nomes de arquivo, modelos ou marcas que a fala costuma errar."}
        </Card>
      ) : (
        <div className="divide-y divide-line border-y border-line">
          {filtered.map((t) => {
            const realIdx = terms.indexOf(t);
            return (
              <div key={`${t.canonical}-${realIdx}`} className="px-2 py-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="break-all font-mono text-[13px] font-medium text-ink">
                        {t.canonical}
                      </span>
                      <span className="rounded-[6px] bg-[#ecece7] px-2 py-0.5 text-[10px] text-[#5d5e58]">
                        {categoryLabel(t.category)}
                      </span>
                      {t.strict && (
                        <span className="rounded-[6px] bg-[#e4efe7] px-2 py-0.5 text-[10px] text-[#25613f]">
                          Literal
                        </span>
                      )}
                      {!t.enabled && (
                        <span className="text-[10px] text-muted">
                          Pausado
                        </span>
                      )}
                    </div>
                    {t.aliases.length > 0 && (
                      <p className="text-[12px] text-muted">
                        Variações: {t.aliases.join(" · ")}
                      </p>
                    )}
                  </div>
                  <div className="flex shrink-0 gap-2">
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => {
                        setEditingIndex(realIdx);
                        setDraft({ ...t, aliases: [...t.aliases] });
                      }}
                    >
                      Editar
                    </Button>
                    <Button
                      variant="danger"
                      size="sm"
                      onClick={() =>
                        void persist(terms.filter((_, i) => i !== realIdx))
                      }
                    >
                      Remover
                    </Button>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/* ------------------------------ Diagnóstico ------------------------------ */

function DiagnosticoTab() {
  const [devMode, setDevModeState] = useState(false);

  useEffect(() => {
    getDevMode().then(setDevModeState).catch(console.error);
  }, []);

  return (
    <div className="space-y-8">
      <div className="divide-y divide-line">
        <Row
          title="Modo desenvolvedor"
          description="Exibe no Histórico os detalhes técnicos, timings e snapshots reais de cada pipeline."
        >
          <Toggle
            label="Ativar modo desenvolvedor"
            checked={devMode}
            onChange={(v) => {
              setDevModeState(v);
              setDevMode(v).catch(console.error);
            }}
          />
        </Row>
      </div>
      <section className="border-t border-line pt-7">
        <h3 className="text-[14px] font-medium text-ink">Logs locais</h3>
        <p className="mt-1 text-[13px] leading-5 text-muted">Informações de execução e falhas ficam somente neste computador.</p>
        <div className="mt-4 rounded-[9px] bg-[#f1f1ec] px-4 py-3 font-mono text-[12px] text-[#4f504b]">
          %APPDATA%\com.haumeavoice.app\logs\
        </div>
        <ul className="mt-4 space-y-2 text-[12px] leading-5 text-muted">
          <li><span className="font-mono text-[#444540]">app.log</span> — eventos e diagnósticos do runtime.</li>
          <li><span className="font-mono text-[#444540]">crash.log</span> — erros não tratados e relatórios de falha.</li>
        </ul>
      </section>
    </div>
  );
}

/* --------------------------------- Shared --------------------------------- */

function Row({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <PreferenceRow title={title} description={description}>{children}</PreferenceRow>
  );
}
