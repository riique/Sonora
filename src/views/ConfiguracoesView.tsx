import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Check, Eye, EyeOff, Plus, X } from "lucide-react";
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
  type EngineConfigSnapshot,
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
import { getThemePreference, setThemePreference, type ThemePreference } from "../lib/theme";
import { Button } from "../components/ui/Button";
import { Input, Select } from "../components/ui/Input";
import { Toggle } from "../components/ui/Toggle";
import { EmptyState, ErrorState, PageHeader, PreferenceRow, RowGroup, Section, Segmented } from "../components/ui/Surface";
import { IntelligenceSettings } from "./IntelligenceSettings";
import { OfflineSection } from "./OfflineSection";
import { UpdatesSection } from "./UpdatesSection";
import { useFeatures } from "../lib/useFeatures";
import { ShortcutSettings } from "./AtalhosView";
import { RecoveryView } from "./RecoveryView";
import type { SettingsTab } from "./index";

const TABS: { key: SettingsTab; label: string; description: string }[] = [
  { key: "geral", label: "Geral", description: "Aparência, atalhos, microfone e armazenamento." },
  { key: "transcricao", label: "Transcrição", description: "Escolha o equilíbrio entre velocidade e precisão." },
  { key: "provedores", label: "Provedores", description: "As chaves ficam neste computador, protegidas pela sua conta do Windows. Uma chave salva pode ser substituída ou removida, mas nunca é exibida de volta." },
  { key: "vocabulario", label: "Vocabulário", description: "Cadastre a grafia correta e como ela costuma soar. A correção só age quando o encaixe é claro; termos literais protegem arquivos, comandos e identificadores." },
  { key: "inteligencia", label: "Saída e contexto", description: "Formatação, destino, styles, snippets e privacidade." },
  { key: "dados", label: "Dados e recuperação", description: "Verificação, itens removidos, backup e diagnóstico." },
];

function displayWindowsPath(path: string): string {
  if (path.startsWith("\\\\?\\UNC\\")) return `\\\\${path.slice(8)}`;
  if (path.startsWith("\\\\?\\")) return path.slice(4);
  return path;
}

export function ConfiguracoesView({ tab: controlledTab, onTabChange }: { tab?: SettingsTab; onTabChange?: (tab: SettingsTab) => void } = {}) {
  const [localTab, setLocalTab] = useState<SettingsTab>("geral");
  const tab = controlledTab ?? localTab;
  const setTab = onTabChange ?? setLocalTab;
  const currentTab = TABS.find((item) => item.key === tab)!;

  return (
    <div>
      <PageHeader title="Ajustes" />
      <div className="settings-layout">
        <nav className="settings-nav scrollbar-thin" aria-label="Seções de ajustes">
          {TABS.map((t) => (
            <button
              key={t.key}
              type="button"
              aria-current={tab === t.key ? "page" : undefined}
              onClick={() => setTab(t.key)}
              className={
                "flex h-8 items-center rounded-[7px] px-2.5 text-left text-[13px] transition-colors " +
                (tab === t.key ? "bg-fill font-medium text-ink" : "text-muted hover:text-ink")
              }
            >
              {t.label}
            </button>
          ))}
        </nav>
        <section className="min-w-0 max-w-[720px]" aria-labelledby={`settings-${tab}`}>
          <header className="mb-10">
            <h2 id={`settings-${tab}`} className="font-display text-[19px] font-semibold tracking-[-0.015em] text-ink">{currentTab.label}</h2>
            <p className="mt-1 max-w-[64ch] text-[13px] leading-5 text-muted">{currentTab.description}</p>
          </header>
          <div key={tab} className="animate-fade-in">
            {tab === "geral" && <GeralTab />}
            {tab === "transcricao" && <TranscricaoTab />}
            {tab === "provedores" && <ProvedoresTab />}
            {tab === "vocabulario" && <VocabularioTab />}
            {tab === "inteligencia" && <IntelligenceSettings />}
            {tab === "dados" && <DadosTab />}
          </div>
        </section>
      </div>
    </div>
  );
}

/* --------------------------------- Geral --------------------------------- */

function GeralTab() {
  const [theme, setTheme] = useState<ThemePreference>(getThemePreference());
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
    invoke<boolean>("get_autostart").then(setStartup).catch((e) => console.error("get_autostart failed:", e));
    getWidgetPreferences().then((preferences) => setWidgetVisibility(preferences.visibility_mode)).catch((e) => console.error("get_widget_preferences failed:", e));
    listAudioDevices().then(setDevices).catch((e) => console.error("listAudioDevices failed:", e));
    getInputDevice().then(setSelectedDevice).catch((e) => console.error("getInputDevice failed:", e));
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
    onMicTestLevel((level) => { if (active) setMicLevel(level); })
      .then((unsub) => { if (active) unlisten = unsub; else unsub(); })
      .catch(() => {});
    return () => {
      active = false;
      unlisten?.();
    };
  }, [isTesting]);

  useEffect(() => () => { stopMicTest().catch(() => {}); }, []);

  const applyStorage = (config: Awaited<ReturnType<typeof setAudioStorageDirectory>>, status: string) => {
    setAudioDirectory(displayWindowsPath(config.effective_directory));
    setDefaultAudioDirectory(displayWindowsPath(config.default_directory));
    setCustomAudioDirectory(Boolean(config.custom_directory));
    setAudioDirectoryStatus(status);
  };

  return (
    <div>
      <Section title="Aparência">
        <RowGroup>
          <PreferenceRow title="Tema" description="Segue o Windows por padrão.">
            <Segmented<ThemePreference>
              label="Tema"
              value={theme}
              onChange={(next) => { setTheme(next); setThemePreference(next); }}
              options={[{ value: "system", label: "Sistema" }, { value: "light", label: "Claro" }, { value: "dark", label: "Escuro" }]}
            />
          </PreferenceRow>
        </RowGroup>
      </Section>

      <Section title="Atalhos globais" description="Funcionam em qualquer aplicativo enquanto o Sonora estiver aberto ou na bandeja. Para trocar um atalho, clique nele e pressione a nova combinação.">
        <ShortcutSettings />
      </Section>

      <Section title="Microfone">
        <RowGroup>
          <PreferenceRow title="Dispositivo de entrada" description="Se o dispositivo sumir, o Sonora usa o padrão do Windows." htmlFor="microphone-input">
            <Select
              id="microphone-input"
              className="w-[240px]"
              value={selectedDevice || "default"}
              onChange={async (e) => {
                const value = e.target.value === "default" ? null : e.target.value;
                setSelectedDevice(value);
                await setInputDevice(value);
                setDevices(await listAudioDevices());
              }}
            >
              <option value="default">Padrão do Windows</option>
              {devices.map((device) => <option key={device} value={device}>{device}</option>)}
            </Select>
          </PreferenceRow>
          <div className="py-4">
            <div className="flex items-center justify-between gap-10">
              <div>
                <h3 className="text-[13px] font-medium text-ink">Testar microfone</h3>
                <p className="mt-0.5 text-[12.5px] text-muted">Fale algo e veja o nível de entrada.</p>
              </div>
              <Button
                size="sm"
                variant={isTesting ? "primary" : "secondary"}
                onClick={async () => {
                  if (isTesting) {
                    await stopMicTest();
                    setIsTesting(false);
                  } else {
                    await startMicTest();
                    setIsTesting(true);
                  }
                }}
              >
                {isTesting && <span className="tally tally--live h-1.5 w-1.5 flex-[0_0_6px]" aria-hidden />}
                {isTesting ? "Parar teste" : "Testar"}
              </Button>
            </div>
            {isTesting && (
              <div className="mt-4 flex items-center gap-4" aria-live="polite">
                <div className="h-1 flex-1 overflow-hidden rounded-full bg-fill">
                  <div className="h-full rounded-full bg-ink transition-[width] duration-75" style={{ width: `${micLevel * 100}%` }} />
                </div>
                <span className="timecode w-10 text-right">{Math.round(micLevel * 100)}%</span>
              </div>
            )}
          </div>
        </RowGroup>
      </Section>

      <Section title="Comportamento" description="Ao fechar a janela, o Sonora continua na bandeja do sistema. Use Sair no menu do ícone para encerrar.">
        <RowGroup>
          <PreferenceRow title="Iniciar com o Windows" description="Abre o Sonora em segundo plano ao ligar o computador.">
            <Toggle label="Iniciar com o Windows" checked={startup} onChange={(v) => { setStartup(v); invoke("set_autostart", { enabled: v }).catch(console.error); }} />
          </PreferenceRow>
          <PreferenceRow title="Mostrar sempre a barra de ditado" description={widgetVisibility === "always" ? "Uma pequena cápsula fica visível mesmo sem ditado ativo." : "A barra aparece só durante gravação, processamento e avisos."}>
            <Toggle
              label="Sempre mostrar a barra de ditado"
              checked={widgetVisibility === "always"}
              onChange={(v) => {
                const mode: WidgetVisibilityMode = v ? "always" : "auto";
                setWidgetVisibility(mode);
                setWidgetVisibilityMode(mode).then((preferences) => setWidgetVisibility(preferences.visibility_mode)).catch(console.error);
              }}
            />
          </PreferenceRow>
        </RowGroup>
      </Section>

      <UpdatesSection />

      <Section title="Pasta dos áudios" description="Onde as próximas gravações são salvas. Áudios existentes continuam onde estão e acessíveis pelo Histórico.">
        <div className="border-y border-line py-4">
          <p className="truncate font-mono text-[12px] text-strong" title={audioDirectory}>{audioDirectory || "Carregando…"}</p>
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
            <span className="meta-label">
              {customAudioDirectory ? "Local personalizado" : "Local padrão do aplicativo"}
              {audioDirectoryStatus && <span className="ml-2 text-cue" role="status">· {audioDirectoryStatus}</span>}
            </span>
            <div className="flex gap-2">
              {customAudioDirectory && (
                <Button
                  size="sm"
                  variant="ghost"
                  title={defaultAudioDirectory}
                  onClick={async () => {
                    try { applyStorage(await setAudioStorageDirectory(null), "Pasta padrão restaurada"); }
                    catch (error) { setAudioDirectoryStatus(String(error)); }
                  }}
                >
                  Usar padrão
                </Button>
              )}
              <Button
                size="sm"
                onClick={async () => {
                  setAudioDirectoryStatus("");
                  const selected = await open({ directory: true, multiple: false, title: "Escolher pasta para os áudios transcritos" });
                  if (typeof selected !== "string") return;
                  try { applyStorage(await setAudioStorageDirectory(selected), "Pasta atualizada"); }
                  catch (error) { setAudioDirectoryStatus(String(error)); }
                }}
              >
                Escolher pasta
              </Button>
            </div>
          </div>
        </div>
      </Section>
    </div>
  );
}

/* ------------------------------- Transcrição ------------------------------- */

const MODES: { id: TranscriptionMode; title: string; engine: string; blurb: string }[] = [
  { id: "ultra-fast", title: "Ultrarrápido", engine: "Whisper via Groq", blurb: "Menor latência" },
  { id: "fast-accurate", title: "Rápido e preciso", engine: "Modelo de áudio", blurb: "Boa precisão com menos etapas" },
  { id: "precise", title: "Preciso", engine: "Whisper + Gemini", blurb: "Melhor equilíbrio geral" },
  { id: "ultra-precise", title: "Ultrapreciso", engine: "Whisper → validador → Gemini", blurb: "Para conteúdo importante" },
];

const META_LANGUAGES = [
  { id: "Portuguese", label: "Português" },
  { id: "English", label: "Inglês" },
  { id: "Spanish", label: "Espanhol" },
  { id: "French", label: "Francês" },
  { id: "Italian", label: "Italiano" },
  { id: "German", label: "Alemão" },
  { id: "Japanese", label: "Japonês" },
];

const SANITIZERS = [
  ["llama-70b", "LLaMA 70B"],
  ["gpt-oss-20b", "GPT-OSS 20B"],
  ["gpt-oss-120b", "GPT-OSS 120B"],
  ["qwen3-27b", "Qwen 3.6 27B"],
] as const;

function TranscricaoTab() {
  const [mode, setMode] = useState<TranscriptionMode>("ultra-fast");
  const [fileTaggingEnabled, setFileTaggingEnabled] = useState(true);
  const [geminiFallback, setGeminiFallback] = useState(true);
  const [sanitizer, setSanitizer] = useState<SanitizerModel>("llama-70b");
  const [sanitizerEnabled, setSanitizerEnabledState] = useState(true);
  const [engineConfig, setEngineConfig] = useState<EngineConfigSnapshot | null>(null);
  const { features, update: updateFeatures, error: featuresError } = useFeatures();
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);
  const [geminiPipelines, setGeminiPipelines] = useState<GeminiPipelineConfig>({
    ultra_fast_whisper: "large-v3-turbo",
    fast_accurate: { model: "flash-lite35", provider: "google-ai-studio", use_custom_model: false, custom_model: "" },
    precise: { model: "flash-lite35", provider: "google-ai-studio", use_custom_model: false, custom_model: "" },
    ultra_precise: { model: "flash-lite35", provider: "google-ai-studio", use_custom_model: false, custom_model: "" },
  });

  const persistMode = (p: { mode?: TranscriptionMode; gemini_fallback_to_whisper?: boolean; file_tagging_enabled?: boolean; gemini_pipelines?: GeminiPipelineConfig }) => {
    updateModeConfig({
      modes_enabled: true,
      mode: p.mode ?? mode,
      gemini_fallback_to_whisper: p.gemini_fallback_to_whisper ?? geminiFallback,
      file_tagging_enabled: p.file_tagging_enabled ?? fileTaggingEnabled,
      gemini_pipelines: p.gemini_pipelines ?? geminiPipelines,
    })
      .then(() => setStatus({ ok: true, text: "Salvo" }))
      .catch((e) => setStatus({ ok: false, text: String(e) }));
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
        setEngineConfig(c);
        if (c.sanitizer) setSanitizer(c.sanitizer as SanitizerModel);
      })
      .catch(console.error);
    getSanitizerEnabled().then(setSanitizerEnabledState).catch(console.error);
  }, []);

  useEffect(() => {
    if (!status?.ok) return;
    const timer = window.setTimeout(() => setStatus(null), 1800);
    return () => window.clearTimeout(timer);
  }, [status]);

  const routeKey = mode === "fast-accurate" ? "fast_accurate" : mode === "precise" ? "precise" : mode === "ultra-precise" ? "ultra_precise" : null;
  const route = routeKey ? geminiPipelines[routeKey] : null;

  const updateRoute = (patch: Partial<GeminiPipelineChoice>, shouldPersist = true) => {
    if (!routeKey) return;
    const next = { ...geminiPipelines, [routeKey]: { ...geminiPipelines[routeKey], ...patch } };
    setGeminiPipelines(next);
    if (shouldPersist) persistMode({ gemini_pipelines: next });
  };

  const selectedLanguages = route?.meta_languages?.length ? route.meta_languages : ["Portuguese", "English"];

  return (
    <div>
      <Section
        title="Modo"
        description="Vale para os próximos ditados e para arquivos enviados."
        action={status && <span role="status" className={"inline-flex items-center gap-1.5 text-[12px] " + (status.ok ? "text-cue" : "text-live")}>{status.ok && <Check className="h-3.5 w-3.5" aria-hidden />}{status.text}</span>}
      >
        <div className="hairline-list border-y border-line" role="radiogroup" aria-label="Modo de transcrição">
          {MODES.map((item) => {
            const active = mode === item.id;
            return (
              <button
                key={item.id}
                type="button"
                role="radio"
                aria-checked={active}
                onClick={() => { setMode(item.id); persistMode({ mode: item.id }); }}
                className="group flex w-full items-center gap-4 py-3.5 text-left"
              >
                <span className={"flex h-4 w-4 shrink-0 items-center justify-center rounded-full border transition-colors " + (active ? "border-ink" : "border-line-strong group-hover:border-faint")} aria-hidden>
                  {active && <span className="h-2 w-2 rounded-full bg-ink" />}
                </span>
                <span className="min-w-0 flex-1">
                  <span className={"block text-[13px] " + (active ? "font-semibold text-ink" : "font-medium text-strong")}>{item.title}</span>
                  <span className="block text-[12.5px] text-muted">{item.blurb}</span>
                </span>
                <span className="timecode shrink-0 max-[640px]:hidden">{item.engine}</span>
              </button>
            );
          })}
        </div>
      </Section>

      <Section title={`Rota do modo ${MODES.find((m) => m.id === mode)?.title ?? ""}`}>
        {routeKey && route ? (
          <>
            <RowGroup>
              <PreferenceRow title="Provedor" htmlFor="route-provider">
                <Select
                  id="route-provider"
                  className="w-[220px]"
                  value={route.provider}
                  onChange={(e) => {
                    const provider = e.target.value as GeminiProvider;
                    if (provider === "meta") updateRoute({ provider, model: "muse-voice-transcribe-1.0", use_custom_model: false });
                    else updateRoute({ provider, model: route.model === "muse-voice-transcribe-1.0" ? "flash-lite35" : route.model });
                  }}
                >
                  <option value="google-ai-studio">Google AI Studio</option>
                  <option value="open-router">OpenRouter</option>
                  {routeKey === "fast_accurate" && <option value="meta">Meta</option>}
                </Select>
              </PreferenceRow>
              <PreferenceRow title="Modelo" htmlFor="route-model">
                <Select
                  id="route-model"
                  className="w-[220px]"
                  value={route.use_custom_model ? "custom" : route.model}
                  onChange={(e) => {
                    if (e.target.value === "custom") updateRoute({ use_custom_model: true }, Boolean(route.custom_model.trim()));
                    else updateRoute({ model: e.target.value as GeminiModel, use_custom_model: false });
                  }}
                >
                  {route.provider === "meta" ? (
                    <option value="muse-voice-transcribe-1.0">Muse Voice Transcribe 1.0</option>
                  ) : (
                    <>
                      <option value="flash-lite35">Gemini 3.5 Flash-Lite</option>
                      <option value="flash36">Gemini 3.6 Flash</option>
                      {routeKey === "fast_accurate" && <option value="transcribe35">Gemini 3.5 Transcribe</option>}
                    </>
                  )}
                  <option value="custom">ID customizado…</option>
                </Select>
              </PreferenceRow>
              {route.use_custom_model && (
                <PreferenceRow title="ID do modelo" htmlFor="route-custom-model">
                  <Input
                    id="route-custom-model"
                    className="w-[260px] font-mono text-[12px]"
                    value={route.custom_model}
                    placeholder={route.provider === "meta" ? "muse-voice-transcribe-1.0" : route.provider === "open-router" ? "google/gemini-3.7-flash" : "gemini-3.7-flash"}
                    onChange={(e) => updateRoute({ custom_model: e.target.value }, false)}
                    onBlur={(e) => { if (e.currentTarget.value.trim()) updateRoute({ custom_model: e.currentTarget.value.trim(), use_custom_model: true }); }}
                  />
                </PreferenceRow>
              )}
              {route.provider === "meta" && (
                <div className="py-4">
                  <h3 className="text-[13px] font-medium text-ink">Idiomas da fala</h3>
                  <p className="mt-0.5 max-w-[60ch] text-[12.5px] leading-5 text-muted">Limitar aos idiomas que você usa evita que o modelo confunda fonemas, e você pode alternar entre eles na mesma fala.</p>
                  <div className="mt-3 flex flex-wrap gap-1.5">
                    {META_LANGUAGES.map((lang) => {
                      const isSelected = selectedLanguages.includes(lang.id);
                      return (
                        <button
                          key={lang.id}
                          type="button"
                          aria-pressed={isSelected}
                          onClick={() => {
                            if (isSelected && selectedLanguages.length === 1) return;
                            updateRoute({ meta_languages: isSelected ? selectedLanguages.filter((l) => l !== lang.id) : [...selectedLanguages, lang.id] });
                          }}
                          className={"inline-flex h-7 items-center gap-1.5 rounded-full border px-3 text-[12px] transition-colors " + (isSelected ? "border-ink bg-ink text-canvas" : "border-line text-muted hover:border-line-strong hover:text-ink")}
                        >
                          {isSelected && <Check className="h-3 w-3" aria-hidden />}
                          {lang.label}
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}
            </RowGroup>
            <p className="mt-3 max-w-[64ch] text-[12px] leading-5 text-muted">
              {route.provider === "meta"
                ? "A Meta Model API usa o Muse Voice Transcribe 1.0, com reconhecimento multilíngue e vocabulário personalizado."
                : route.provider === "open-router"
                  ? "A rota é automática: modelos dedicados de fala usam Speech-to-Text; modelos com áudio usam Chat Completions."
                  : "O Google AI Studio usa modelos multimodais com áudio pela Gemini API."}
            </p>
          </>
        ) : (
          <>
            <RowGroup>
              <PreferenceRow title="Modelo Whisper" description="Endpoint de transcrição do OpenRouter, com o Groq fixo e sem fallback." htmlFor="ultra-fast-whisper-model">
                <Select
                  id="ultra-fast-whisper-model"
                  className="w-[240px] font-mono text-[12px]"
                  value={geminiPipelines.ultra_fast_whisper}
                  onChange={(e) => {
                    const next = { ...geminiPipelines, ultra_fast_whisper: e.target.value as OpenRouterWhisperModel };
                    setGeminiPipelines(next);
                    persistMode({ gemini_pipelines: next });
                  }}
                >
                  <option value="large-v3-turbo">whisper-large-v3-turbo</option>
                  <option value="large-v3">whisper-large-v3</option>
                </Select>
              </PreferenceRow>
              <PreferenceRow
                title="Refinar com IA"
                description="Corrige pontuação e termos do vocabulário e aplica o estilo do aplicativo em uso. Acrescenta cerca de meio segundo. Desligado, o texto sai como o Whisper ouviu."
              >
                <Toggle
                  label="Refinar com IA no Ultrarrápido"
                  checked={features?.quick_refine ?? true}
                  disabled={!features}
                  onChange={(value) => void updateFeatures({ quick_refine: value })}
                />
              </PreferenceRow>
            </RowGroup>
            {featuresError && <p className="mt-2 text-[12px] text-live" role="alert">{featuresError}</p>}
          </>
        )}
      </Section>

      {(mode === "fast-accurate" || mode === "ultra-precise") && (
        <Section title="Segurança da rota">
          <RowGroup>
            {mode === "fast-accurate" && (
              <PreferenceRow title="Usar Whisper se o modelo de áudio falhar" description="O histórico marca quando o fallback acontecer.">
                <Toggle label="Usar Whisper se o modelo de áudio falhar" checked={geminiFallback} onChange={(v) => { setGeminiFallback(v); persistMode({ gemini_fallback_to_whisper: v }); }} />
              </PreferenceRow>
            )}
            {mode === "ultra-precise" && (
              <>
                <PreferenceRow title="Validador semântico" description="Corrige a ortografia depois do Whisper, antes do Gemini.">
                  <Toggle label="Ativar validador semântico" checked={sanitizerEnabled} onChange={(v) => { setSanitizerEnabledState(v); setSanitizerEnabled(v).catch(console.error); }} />
                </PreferenceRow>
                {sanitizerEnabled && (
                  <PreferenceRow title="Modelo do validador" htmlFor="sanitizer-model">
                    <Select
                      id="sanitizer-model"
                      className="w-[200px]"
                      value={sanitizer}
                      onChange={(e) => {
                        const id = e.target.value as SanitizerModel;
                        setSanitizer(id);
                        if (engineConfig) {
                          invoke("update_engine_config", { payload: { ...engineConfig, sanitizer: id } })
                            .then(() => setEngineConfig({ ...engineConfig, sanitizer: id }))
                            .catch(console.error);
                        }
                      }}
                    >
                      {SANITIZERS.map(([id, label]) => <option key={id} value={id}>{label}</option>)}
                    </Select>
                  </PreferenceRow>
                )}
              </>
            )}
          </RowGroup>
        </Section>
      )}

      <OfflineSection features={features} onChange={updateFeatures} />

      <Section title="Para programação">
        <RowGroup>
          <PreferenceRow title="FileTagging" description="Converte referências claras a arquivos em menções como @index.tsx, para chats de código.">
            <Toggle label="Ativar FileTagging" checked={fileTaggingEnabled} onChange={(value) => { setFileTaggingEnabled(value); persistMode({ file_tagging_enabled: value }); }} />
          </PreferenceRow>
        </RowGroup>
      </Section>
    </div>
  );
}

/* ------------------------------- Provedores ------------------------------- */

type KeyId = "groq" | "google" | "deepgram" | "openrouter" | "meta";

const PROVIDERS: { id: KeyId; name: string; placeholder: string; usedBy: string }[] = [
  { id: "openrouter", name: "OpenRouter", placeholder: "sk-or-v1-…", usedBy: "Ultrarrápido, rotas OpenRouter e retrato de voz" },
  { id: "google", name: "Google Gemini", placeholder: "AIza…", usedBy: "Rápido e preciso, Preciso, Ultrapreciso e pronúncia" },
  { id: "groq", name: "Groq", placeholder: "gsk_…", usedBy: "Whisper, validador e fallbacks" },
  { id: "meta", name: "Meta Model API", placeholder: "Chave da Meta (dev.meta.ai)", usedBy: "Rápido e preciso com provedor Meta" },
];

function ProvedoresTab() {
  const [keys, setKeys] = useState<Record<KeyId, string[]>>({ groq: [], google: [], deepgram: [], openrouter: [], meta: [] });
  const [visible, setVisible] = useState<Record<string, boolean>>({});
  const [saving, setSaving] = useState<KeyId | null>(null);
  const [saved, setSaved] = useState<KeyId | null>(null);
  const [error, setError] = useState("");
  const [managing, setManaging] = useState<KeyId | null>(null);

  useEffect(() => {
    getApiKeys()
      .then((k) => setKeys({ groq: k.groq ?? [], google: k.google ?? [], deepgram: k.deepgram ?? [], openrouter: k.openrouter ?? [], meta: k.meta ?? [] }))
      .catch(console.error);
  }, []);

  const save = async (id: KeyId) => {
    setSaving(id);
    setError("");
    try {
      const current = await getApiKeys();
      await saveApiKeys({ ...current, [id]: keys[id] });
      const stored = await getApiKeys();
      setKeys((draft) => ({ ...draft, [id]: stored[id] ?? [] }));
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
    <div>
      {error && <ErrorState>{error}</ErrorState>}
      <div className="hairline-list border-y border-line">
        {PROVIDERS.map((provider) => {
          const count = keys[provider.id].filter((key) => key.trim()).length;
          const isManaging = managing === provider.id;
          return (
            <section key={provider.id} className="py-4">
              <div className="flex items-center justify-between gap-6">
                <div className="min-w-0">
                  <div className="flex items-center gap-2.5">
                    <h3 className="text-[13px] font-medium text-ink">{provider.name}</h3>
                    <span className={"inline-flex items-center gap-1.5 text-[11.5px] " + (count ? "text-cue" : "text-muted")}>
                      <span className={"h-1.5 w-1.5 rounded-full " + (count ? "bg-cue" : "bg-line-strong")} aria-hidden />
                      {count ? `${count} ${count === 1 ? "chave" : "chaves"}` : "Sem chave"}
                    </span>
                  </div>
                  <p className="mt-0.5 truncate text-[12.5px] text-muted">{provider.usedBy}</p>
                </div>
                <Button size="sm" variant={isManaging ? "ghost" : "secondary"} onClick={() => setManaging(isManaging ? null : provider.id)} aria-expanded={isManaging}>
                  {isManaging ? "Fechar" : count ? "Gerenciar" : "Adicionar chave"}
                </Button>
              </div>
              {isManaging && (
                <div className="mt-4 animate-fade-in space-y-2">
                  {(keys[provider.id].length ? keys[provider.id] : [""]).map((key, index) => {
                    const visibilityKey = `${provider.id}-${index}`;
                    const updateKey = (value: string) => setKeys((current) => {
                      const list = current[provider.id].length ? [...current[provider.id]] : [""];
                      list[index] = value;
                      return { ...current, [provider.id]: list };
                    });
                    return (
                      <div key={visibilityKey} className="flex gap-1.5">
                        <div className="relative flex-1">
                          <Input
                            name={`${provider.id}-api-key-${index + 1}`}
                            type={visible[visibilityKey] ? "text" : "password"}
                            placeholder={key.startsWith("stored:") ? "Chave protegida · digite para substituir" : provider.placeholder}
                            value={key.startsWith("stored:") ? "" : key}
                            onChange={(event) => updateKey(event.target.value)}
                            autoComplete="off"
                            spellCheck={false}
                            className="pr-10 font-mono text-[12px]"
                            aria-label={`Chave ${index + 1} de ${provider.name}`}
                          />
                          <button type="button" className="icon-button absolute right-0.5 top-0.5" onClick={() => setVisible((current) => ({ ...current, [visibilityKey]: !current[visibilityKey] }))} aria-label={visible[visibilityKey] ? "Ocultar chave" : "Mostrar chave"}>
                            {visible[visibilityKey] ? <EyeOff className="h-4 w-4" aria-hidden /> : <Eye className="h-4 w-4" aria-hidden />}
                          </button>
                        </div>
                        <button type="button" onClick={() => setKeys((current) => ({ ...current, [provider.id]: current[provider.id].filter((_, itemIndex) => itemIndex !== index) }))} className="icon-button h-9 w-9 hover:text-live" aria-label={`Remover chave ${index + 1} de ${provider.name}`} title="Remover chave">
                          <X className="h-4 w-4" aria-hidden />
                        </button>
                      </div>
                    );
                  })}
                  <div className="flex items-center justify-between gap-3 pt-2">
                    <Button size="sm" variant="ghost" onClick={() => setKeys((current) => ({ ...current, [provider.id]: [...(current[provider.id].length ? current[provider.id] : [""]), ""] }))}>
                      <Plus className="h-3.5 w-3.5" aria-hidden />Outra chave
                    </Button>
                    <Button size="sm" variant="primary" disabled={saving !== null} onClick={() => save(provider.id)}>
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
  return { canonical: "", aliases: [], category: "other", strict: false, enabled: true };
}

function VocabularioTab() {
  const [terms, setTerms] = useState<VocabularyTerm[]>([]);
  const [query, setQuery] = useState("");
  const [draft, setDraft] = useState<VocabularyTerm>(emptyTerm());
  const [aliasDraft, setAliasDraft] = useState("");
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    getVocabulary().then(setTerms).catch(console.error).finally(() => setLoading(false));
  }, []);

  const closeForm = () => {
    setFormOpen(false);
    setEditingIndex(null);
    setDraft(emptyTerm());
    setAliasDraft("");
    setError("");
  };

  const persist = async (next: VocabularyTerm[]) => {
    setSaving(true);
    setError("");
    try {
      setTerms(await setVocabulary(next));
      closeForm();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setSaving(false);
    }
  };

  const addAlias = () => {
    const alias = aliasDraft.trim();
    if (!alias) return;
    setDraft((d) => ({ ...d, aliases: d.aliases.includes(alias) ? d.aliases : [...d.aliases, alias] }));
    setAliasDraft("");
  };

  const filtered = query.trim()
    ? terms.filter((t) => {
        const q = query.trim().toLowerCase();
        return t.canonical.toLowerCase().includes(q) || t.aliases.some((a) => a.toLowerCase().includes(q)) || t.category.includes(q);
      })
    : terms;

  const categoryLabel = (id: VocabularyCategory) => VOCAB_CATEGORIES.find((c) => c.id === id)?.label ?? id;

  return (
    <div>

      {formOpen ? (
        <section className="mb-10 animate-fade-in border-y border-line py-6" aria-labelledby="vocab-form-title">
          <h3 id="vocab-form-title" className="section-title">{editingIndex !== null ? "Editar termo" : "Novo termo"}</h3>
          <div className="mt-4 grid grid-cols-[minmax(0,1fr)_200px] gap-4 max-[640px]:grid-cols-1">
            <div>
              <label htmlFor="vocabulary-canonical" className="field-label">Grafia correta</label>
              <Input id="vocabulary-canonical" autoFocus placeholder="provider-routing.json" value={draft.canonical} onChange={(e) => setDraft((d) => ({ ...d, canonical: e.target.value }))} spellCheck={false} />
            </div>
            <div>
              <label htmlFor="vocabulary-category" className="field-label">Categoria</label>
              <Select id="vocabulary-category" wrapperClassName="w-full" className="w-full" value={draft.category} onChange={(e) => setDraft((d) => ({ ...d, category: e.target.value as VocabularyCategory }))}>
                {VOCAB_CATEGORIES.map((c) => <option key={c.id} value={c.id}>{c.label}</option>)}
              </Select>
            </div>
          </div>
          <div className="mt-4">
            <label htmlFor="vocabulary-alias" className="field-label">Como costuma soar <span className="font-normal text-muted">· Enter para incluir</span></label>
            <div className="flex gap-2">
              <Input id="vocabulary-alias" placeholder="provider routing json" value={aliasDraft} onChange={(e) => setAliasDraft(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); addAlias(); } }} spellCheck={false} />
              <Button onClick={addAlias} disabled={!aliasDraft.trim()}>Incluir</Button>
            </div>
            {draft.aliases.length > 0 && (
              <div className="mt-2.5 flex flex-wrap gap-1.5">
                {draft.aliases.map((a) => (
                  <span key={a} className="inline-flex h-7 items-center gap-1 rounded-full bg-fill pl-3 pr-1.5 text-[12px] text-strong">
                    {a}
                    <button type="button" className="flex h-5 w-5 items-center justify-center rounded-full text-muted hover:bg-line hover:text-ink" onClick={() => setDraft((d) => ({ ...d, aliases: d.aliases.filter((x) => x !== a) }))} aria-label={`Remover ${a}`}>
                      <X className="h-3 w-3" aria-hidden />
                    </button>
                  </span>
                ))}
              </div>
            )}
          </div>
          <div className="mt-5 flex flex-wrap items-center justify-between gap-4">
            <div className="flex items-center gap-5">
              <label className="flex cursor-pointer items-center gap-2 text-[12.5px] text-strong"><input type="checkbox" checked={draft.strict} onChange={(e) => setDraft((d) => ({ ...d, strict: e.target.checked }))} />Literal</label>
              <label className="flex cursor-pointer items-center gap-2 text-[12.5px] text-strong"><input type="checkbox" checked={draft.enabled} onChange={(e) => setDraft((d) => ({ ...d, enabled: e.target.checked }))} />Ativo</label>
            </div>
            <div className="flex gap-2">
              <Button variant="ghost" onClick={closeForm}>Cancelar</Button>
              <Button
                variant="primary"
                disabled={saving || !draft.canonical.trim()}
                onClick={() => {
                  const canonical = draft.canonical.trim();
                  if (!canonical) { setError("Informe a grafia correta."); return; }
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
                {editingIndex !== null ? "Salvar" : "Adicionar"}
              </Button>
            </div>
          </div>
          {error && <p className="mt-3 text-[12.5px] text-live" role="alert">{error}</p>}
        </section>
      ) : null}

      <div className="mb-4 flex items-center gap-3">
        {terms.length > 0 && <Input type="search" aria-label="Buscar no vocabulário" placeholder="Buscar termo ou variação" value={query} onChange={(e) => setQuery(e.target.value)} />}
        {!formOpen && <Button className={terms.length ? "" : "ml-0"} onClick={() => { setDraft(emptyTerm()); setEditingIndex(null); setFormOpen(true); }}><Plus className="h-4 w-4" aria-hidden />Novo termo</Button>}
      </div>

      {loading ? (
        <p className="py-6 text-[13px] text-muted">Carregando…</p>
      ) : filtered.length === 0 ? (
        <div className="border-t border-line">
          <EmptyState
            title={query.trim() ? "Nenhum termo encontrado" : "Nenhum termo ainda"}
            description={query.trim() ? "Tente outra grafia ou variação." : "Cadastre nomes de arquivo, modelos ou marcas que a fala costuma errar."}
          />
        </div>
      ) : (
        <ul className="hairline-list border-y border-line">
          {filtered.map((t) => {
            const realIdx = terms.indexOf(t);
            return (
              <li key={`${t.canonical}-${realIdx}`} className="group flex items-start justify-between gap-4 py-3.5">
                <div className={"min-w-0 " + (t.enabled ? "" : "opacity-55")}>
                  <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                    <span className="break-all font-mono text-[12.5px] font-medium text-ink">{t.canonical}</span>
                    <span className="meta-label">{categoryLabel(t.category)}{t.strict ? " · literal" : ""}{!t.enabled ? " · pausado" : ""}</span>
                  </div>
                  {t.aliases.length > 0 && <p className="mt-1 text-[12.5px] text-muted">soa como {t.aliases.map((a) => `“${a}”`).join(", ")}</p>}
                </div>
                <div className="reveal-on-row flex shrink-0 gap-1">
                  <Button variant="ghost" size="sm" onClick={() => { setEditingIndex(realIdx); setDraft({ ...t, aliases: [...t.aliases] }); setFormOpen(true); }}>Editar</Button>
                  <Button variant="danger" size="sm" onClick={() => { if (window.confirm(`Remover “${t.canonical}” do vocabulário?`)) void persist(terms.filter((_, i) => i !== realIdx)); }}>Remover</Button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

/* --------------------------- Dados e recuperação --------------------------- */

function DadosTab() {
  const [devMode, setDevModeState] = useState(false);

  useEffect(() => {
    getDevMode().then(setDevModeState).catch(console.error);
  }, []);

  return (
    <div>
      <RecoveryView />
      <Section title="Diagnóstico">
        <RowGroup>
          <PreferenceRow title="Modo desenvolvedor" description="Mostra no Histórico tempos, tentativas e requisições sanitizadas de cada ditado.">
            <Toggle label="Ativar modo desenvolvedor" checked={devMode} onChange={(v) => { setDevModeState(v); setDevMode(v).catch(console.error); }} />
          </PreferenceRow>
          <div className="py-4">
            <h3 className="text-[13px] font-medium text-ink">Logs locais</h3>
            <p className="mt-0.5 text-[12.5px] text-muted">Ficam somente neste computador: <span className="font-mono text-strong">app.log</span> para eventos e <span className="font-mono text-strong">crash.log</span> para falhas.</p>
            <p className="mt-2 font-mono text-[12px] text-strong">%APPDATA%\com.haumeavoice.app\logs\</p>
          </div>
        </RowGroup>
      </Section>
    </div>
  );
}
