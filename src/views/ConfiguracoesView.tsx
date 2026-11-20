import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
  getVocabulary,
  setVocabulary,
  getVocabularySuggestions,
  resolveVocabularySuggestion,
  getRunAsAdmin,
  setRunAsAdmin,
  type RunAsAdminStatus,
  getTranscriptionRoute,
  setTranscriptionRoute,
  getApiKeys,
  saveApiKeys,
  type CorrectionEvent,
  type MicReadyMode,
  type TranscriptionModel,
  type TranscriptionProvider,
  type TranscriptionRoute,
  type VocabularyTerm,
  type VocabularyCategory,
  type WidgetVisibilityMode,
} from "../lib/tauri";
import { getThemePreference, setThemePreference, type ThemePreference } from "../lib/theme";
import { Button } from "../components/ui/Button";
import { Input, Select } from "../components/ui/Input";
import { Toggle } from "../components/ui/Toggle";
import { EmptyState, ErrorState, PageHeader, PreferenceRow, RowGroup, Section, Segmented } from "../components/ui/Surface";
import { UpdatesSection } from "./UpdatesSection";
import { useFeatures } from "../lib/useFeatures";
import { ShortcutSettings } from "./AtalhosView";
import type { SettingsTab } from "./index";

const TABS: { key: SettingsTab; label: string; description: string }[] = [
  { key: "geral", label: "Geral", description: "Aparência, atalhos, microfone e atualizações." },
  { key: "transcricao", label: "Transcrição", description: "Modelo usado nos ditados e chaves de API." },
  { key: "vocabulario", label: "Vocabulário", description: "Cadastre a grafia correta e como ela costuma soar. A correção só age quando o encaixe é claro; termos literais protegem arquivos, comandos e identificadores." },
];

const MIC_READY_HINTS: Record<MicReadyMode, string> = {
  off: "O microfone só abre ao gravar. Alguns headsets sem fio cortam as primeiras palavras.",
  after_use: "Fica aberto por 10 minutos depois de cada ditado, para não perder o começo da fala.",
  always: "Fica aberto enquanto o Sonora estiver rodando. O Windows mostra o microfone em uso.",
};

export function ConfiguracoesView({ tab: controlledTab, onTabChange }: { tab?: SettingsTab; onTabChange?: (tab: SettingsTab) => void } = {}) {
  const [localTab, setLocalTab] = useState<SettingsTab>("geral");
  const tab = controlledTab ?? localTab;
  const setTab = onTabChange ?? setLocalTab;
  const currentTab = TABS.find((item) => item.key === tab) ?? TABS[0];

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
            {tab === "vocabulario" && <VocabularioTab />}
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
  const [runAsAdmin, setRunAsAdminState] = useState<RunAsAdminStatus | null>(null);
  const [runAsAdminError, setRunAsAdminError] = useState("");
  const [widgetVisibility, setWidgetVisibility] = useState<WidgetVisibilityMode>("auto");
  const [devices, setDevices] = useState<string[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null);
  const [isTesting, setIsTesting] = useState(false);
  const [micLevel, setMicLevel] = useState(0);
  const { features, update: updateFeatures, error: featuresError } = useFeatures();
  const micReady = features?.mic_ready ?? "after_use";

  useEffect(() => {
    invoke<boolean>("get_autostart").then(setStartup).catch((e) => console.error("get_autostart failed:", e));
    getRunAsAdmin().then(setRunAsAdminState).catch((e) => console.error("get_run_as_admin failed:", e));
    getWidgetPreferences().then((preferences) => setWidgetVisibility(preferences.visibility_mode)).catch((e) => console.error("get_widget_preferences failed:", e));
    listAudioDevices().then(setDevices).catch((e) => console.error("listAudioDevices failed:", e));
    getInputDevice().then(setSelectedDevice).catch((e) => console.error("getInputDevice failed:", e));
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
          <PreferenceRow title="Manter microfone pronto" description={featuresError || MIC_READY_HINTS[micReady]}>
            <Segmented<MicReadyMode>
              label="Manter microfone pronto"
              value={micReady}
              onChange={(next) => void updateFeatures({ mic_ready: next })}
              options={[{ value: "off", label: "Desligado" }, { value: "after_use", label: "Após uso" }, { value: "always", label: "Sempre" }]}
            />
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
          <PreferenceRow
            title="Executar como administrador"
            description={runAsAdminError || (runAsAdmin?.enabled && !runAsAdmin.elevated
              ? "Ativado, mas o Sonora está sem privilégios agora. Reinicie e confirme o aviso do Windows."
              : "Permite colar automaticamente em aplicativos abertos como administrador. O Windows pede confirmação ao abrir o Sonora.")}
          >
            <Toggle
              label="Executar como administrador"
              checked={runAsAdmin?.enabled ?? false}
              onChange={(v) => {
                setRunAsAdminError("");
                setRunAsAdminState((previous) => ({ elevated: previous?.elevated ?? false, enabled: v }));
                setRunAsAdmin(v).catch((e) => {
                  setRunAsAdminState((previous) => ({ elevated: previous?.elevated ?? false, enabled: false }));
                  setRunAsAdminError(String(e?.message ?? e));
                });
              }}
            />
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
    </div>
  );
}

/* ------------------------------- Transcrição ------------------------------- */

const DIRECT_ASSEMBLYAI_MODELS: { id: TranscriptionModel; label: string }[] = [
  { id: "assemblyai-stt-3-7", label: "AssemblyAI-STT-3.7 (assemblyai-stt-3-7)" },
];

const DIRECT_ELEVENLABS_MODELS: { id: TranscriptionModel; label: string }[] = [
  { id: "elevenlabs-scribe-v3", label: "ElevenLabs Scribe v3 (scribe_v3)" },
];

const OPENROUTER_MODELS: { id: TranscriptionModel; label: string }[] = [
  { id: "assemblyai-universal-3-6-pro", label: "AssemblyAI Universal-3.6 Pro" },
  { id: "elevenlabs-scribe-v2", label: "ElevenLabs Scribe v2" },
];

const CUSTOM_MODEL_OPTION = "__custom__";

function TranscricaoTab() {
  const [route, setRoute] = useState<TranscriptionRoute | null>(null);
  const [customDraft, setCustomDraft] = useState("");
  const [customOpen, setCustomOpen] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    getTranscriptionRoute()
      .then((loaded) => {
        setRoute(loaded);
        setCustomDraft(loaded.custom_model ?? "");
        setCustomOpen(loaded.provider === "open-router" && Boolean(loaded.use_custom_model));
      })
      .catch((e) => setStatus({ ok: false, text: String(e) }));
  }, []);

  useEffect(() => {
    if (!status?.ok) return;
    const timer = window.setTimeout(() => setStatus(null), 1800);
    return () => window.clearTimeout(timer);
  }, [status]);

  const persistRoute = (next: TranscriptionRoute) => {
    setRoute(next);
    setTranscriptionRoute(next)
      .then((saved) => {
        setRoute(saved);
        setCustomDraft(saved.custom_model ?? "");
        setCustomOpen(saved.provider === "open-router" && Boolean(saved.use_custom_model));
        setStatus({ ok: true, text: "Salvo" });
      })
      .catch((e) => setStatus({ ok: false, text: String(e) }));
  };

  const saveCustomModel = () => {
    if (!route) return;
    persistRoute({
      ...route,
      provider: "open-router",
      use_custom_model: true,
      custom_model: customDraft.trim(),
    });
  };

  const selectedModelValue = route?.provider === "open-router" && customOpen
    ? CUSTOM_MODEL_OPTION
    : route?.model ?? "assemblyai-stt-3-7";

  const availableModels = route?.provider === "open-router"
    ? OPENROUTER_MODELS
    : route?.provider === "elevenlabs"
      ? DIRECT_ELEVENLABS_MODELS
      : DIRECT_ASSEMBLYAI_MODELS;

  return (
    <div>
      <Section
        title="Modelo"
        description="Use AssemblyAI-STT-3.7 ou ElevenLabs Scribe v3 diretamente pela API oficial, ou compare com modelos do OpenRouter (Universal-3.6 Pro ou ElevenLabs Scribe v2) e qualquer ID customizado compatível. Em caso de falha do provedor, o Whisper pelo OpenRouter atua como reserva."
        action={status && <span role="status" className={"inline-flex items-center gap-1.5 text-[12px] " + (status.ok ? "text-cue" : "text-live")}>{status.ok && <Check className="h-3.5 w-3.5" aria-hidden />}{status.text}</span>}
      >
        {route ? (
          <RowGroup>
            <PreferenceRow title="Provedor" htmlFor="route-provider">
              <Select
                id="route-provider"
                className="w-[240px]"
                value={route.provider}
                onChange={(e) => {
                  const provider = e.target.value as TranscriptionProvider;
                  if (provider === "assemblyai" || provider === "elevenlabs") {
                    setCustomOpen(false);
                    persistRoute({
                      ...route,
                      provider,
                      model: provider === "assemblyai" ? "assemblyai-stt-3-7" : "elevenlabs-scribe-v3",
                      use_custom_model: false,
                    });
                  } else {
                    const nextModel = OPENROUTER_MODELS.some((m) => m.id === route.model)
                      ? route.model
                      : "assemblyai-universal-3-6-pro";
                    setCustomOpen(false);
                    persistRoute({
                      ...route,
                      provider,
                      model: nextModel,
                      use_custom_model: false,
                    });
                  }
                }}
              >
                <option value="assemblyai">AssemblyAI (API direta)</option>
                <option value="elevenlabs">ElevenLabs (API direta)</option>
                <option value="open-router">OpenRouter</option>
              </Select>
            </PreferenceRow>
            <PreferenceRow title="Modelo" htmlFor="route-model">
              <Select
                id="route-model"
                className="w-[240px]"
                value={selectedModelValue}
                onChange={(e) => {
                  const value = e.target.value;
                  if (value === CUSTOM_MODEL_OPTION) {
                    setCustomOpen(true);
                    setStatus(null);
                    return;
                  }
                  setCustomOpen(false);
                  persistRoute({
                    ...route,
                    model: value as TranscriptionModel,
                    use_custom_model: false,
                  });
                }}
              >
                {availableModels.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
                {route.provider === "open-router" && (
                  <option value={CUSTOM_MODEL_OPTION}>ID customizado do OpenRouter…</option>
                )}
              </Select>
            </PreferenceRow>
            {route.provider === "open-router" && customOpen && (
              <PreferenceRow
                title="ID customizado"
                description="Informe qualquer ID compatível no formato provedor/modelo (ex.: assemblyai/universal-3-6-pro ou elevenlabs/scribe-v2)."
                htmlFor="route-custom-model"
              >
                <div className="flex w-[320px] max-w-full gap-2">
                  <Input
                    id="route-custom-model"
                    placeholder="provedor/modelo"
                    value={customDraft}
                    onChange={(e) => setCustomDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        saveCustomModel();
                      }
                    }}
                    spellCheck={false}
                    className="font-mono text-[12px]"
                  />
                  <Button size="sm" variant="primary" disabled={!customDraft.trim()} onClick={saveCustomModel}>
                    Salvar
                  </Button>
                </div>
              </PreferenceRow>
            )}
          </RowGroup>
        ) : (
          <p className="text-[13px] text-muted">Carregando…</p>
        )}
      </Section>

      <Section title="Chaves de API" description="Ficam neste computador, protegidas pela sua conta do Windows. Uma chave salva pode ser substituída ou removida, mas nunca é exibida de volta.">
        <ProvedoresList />
      </Section>
    </div>
  );
}

/* ------------------------------- Provedores ------------------------------- */

type KeyId = "assemblyai" | "elevenlabs" | "openrouter";

const PROVIDERS: { id: KeyId; name: string; placeholder: string; usedBy: string }[] = [
  { id: "assemblyai", name: "AssemblyAI", placeholder: "Chave da API da AssemblyAI…", usedBy: "Transcrição direta com AssemblyAI-STT-3.7" },
  { id: "elevenlabs", name: "ElevenLabs", placeholder: "Chave da API da ElevenLabs…", usedBy: "Transcrição direta com ElevenLabs Scribe v3" },
  { id: "openrouter", name: "OpenRouter", placeholder: "sk-or-v1-…", usedBy: "Universal-3.6 Pro, Scribe v2, IDs customizados, reserva Whisper, comando de voz e retrato da voz" },
];

function ProvedoresList() {
  const [keys, setKeys] = useState<Record<KeyId, string[]>>({ assemblyai: [], elevenlabs: [], openrouter: [] });
  const [visible, setVisible] = useState<Record<string, boolean>>({});
  const [saving, setSaving] = useState<KeyId | null>(null);
  const [saved, setSaved] = useState<KeyId | null>(null);
  const [error, setError] = useState("");
  const [managing, setManaging] = useState<KeyId | null>(null);

  useEffect(() => {
    getApiKeys()
      .then((k) => setKeys({ assemblyai: k.assemblyai ?? [], elevenlabs: k.elevenlabs ?? [], openrouter: k.openrouter ?? [] }))
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

  const [suggestions, setSuggestions] = useState<CorrectionEvent[]>([]);

  useEffect(() => {
    getVocabulary().then(setTerms).catch(console.error).finally(() => setLoading(false));
    getVocabularySuggestions().then(setSuggestions).catch(console.error);
  }, []);

  const resolveSuggestion = (id: string, accept: boolean) => {
    resolveVocabularySuggestion(id, accept)
      .then(async () => {
        setSuggestions((items) => items.filter((item) => item.id !== id));
        if (accept) setTerms(await getVocabulary());
      })
      .catch((e) => setError(String(e)));
  };

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
      {suggestions.length > 0 && (
        <section className="mb-10" aria-labelledby="vocab-suggestions-title">
          <h3 id="vocab-suggestions-title" className="section-title">Sugestões das suas correções</h3>
          <p className="mt-1 text-[12.5px] text-muted">Palavras que você corrigiu mais de uma vez no Histórico.</p>
          <div className="hairline-list mt-3 border-y border-line">
            {suggestions.map((event) => (
              <div key={event.id} className="flex items-center justify-between gap-5 py-3">
                <p className="text-[13px] text-ink"><span className="text-muted line-through decoration-line-strong">{event.before}</span> → <span className="font-medium">{event.after}</span><span className="timecode ml-2">{event.count}×</span></p>
                <div className="flex gap-1">
                  <Button size="sm" variant="ghost" onClick={() => resolveSuggestion(event.id, false)}>Ignorar</Button>
                  <Button size="sm" onClick={() => resolveSuggestion(event.id, true)}><Check className="h-3.5 w-3.5" aria-hidden />Adicionar</Button>
                </div>
              </div>
            ))}
          </div>
        </section>
      )}

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
