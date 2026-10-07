import { useCallback, useEffect, useRef, useState } from "react";
import { AlertCircle, Check, Loader2, Plus, Trash2 } from "lucide-react";
import {
  getContextPreferences, getOutputPolicyConfig, getSnippets, getVocabularySuggestions,
  resolveVocabularySuggestion, setContextPreferences, setOutputPolicyConfig, setSnippets,
  type ContextPreferences, type ContextSourceKind, type CorrectionEvent, type OutputPolicyConfig,
  type OutputProfile, type VoiceSnippet,
} from "../lib/tauri";
import { Button } from "../components/ui/Button";
import { Input, Select, Textarea } from "../components/ui/Input";
import { Toggle } from "../components/ui/Toggle";
import { ErrorState, PreferenceRow, RowGroup, Section } from "../components/ui/Surface";
import { useFeatures } from "../lib/useFeatures";

const sourceLabels: Record<ContextSourceKind, string> = {
  application: "Aplicativo", window_title: "Título da janela", domain: "Domínio",
  selection: "Seleção", caret_context: "Texto próximo ao cursor", clipboard: "Clipboard",
};
const split = (value: string) => value.split(",").map((item) => item.trim()).filter(Boolean);
type IntelligenceSnapshot = { context: ContextPreferences; policy: OutputPolicyConfig; snippets: VoiceSnippet[] };
type SaveStatus = "idle" | "pending" | "saving" | "saved" | "invalid" | "error";
const fingerprint = ({ context, policy, snippets }: IntelligenceSnapshot) => JSON.stringify({ context, policy, snippets });
const validateSnapshot = ({ context, policy, snippets }: IntelligenceSnapshot) => {
  if (!Number.isFinite(context.max_context_chars) || context.max_context_chars < 100 || context.max_context_chars > 4_000) return "O limite por fonte deve ficar entre 100 e 4.000 caracteres.";
  const profileIds = new Set<string>();
  for (const profile of policy.profiles) {
    const id = profile.id.trim().toLowerCase();
    if (!id || profileIds.has(id)) return "Cada style precisa ter um ID preenchido e exclusivo.";
    if (!profile.name.trim()) return "Cada style precisa ter um nome.";
    if ((profile.style_instruction?.length ?? 0) > 2_000) return "A instrução de style pode ter no máximo 2.000 caracteres.";
    profileIds.add(id);
  }
  if (policy.temporary_override && !policy.profiles.some((profile) => profile.enabled && profile.id === policy.temporary_override)) return "O override temporário precisa apontar para um style ativo.";
  const triggers = new Set<string>();
  for (const snippet of snippets) {
    const trigger = snippet.trigger.trim().toLowerCase().replace(/\s+/g, " ");
    if (!snippet.id.trim() || !trigger || !snippet.expansion) return "Preencha o trigger e a expansão do novo snippet para salvá-lo.";
    if (triggers.has(trigger)) return "Cada snippet precisa ter um trigger exclusivo.";
    triggers.add(trigger);
  }
  return null;
};

export function IntelligenceSettings() {
  const { features, update: updateFeatures } = useFeatures();
  const [context, setContext] = useState<ContextPreferences | null>(null);
  const [policy, setPolicy] = useState<OutputPolicyConfig | null>(null);
  const [snippets, setSnippetItems] = useState<VoiceSnippet[]>([]);
  const [suggestions, setSuggestions] = useState<CorrectionEvent[]>([]);
  const [saveStatus, setSaveStatus] = useState<SaveStatus>("idle");
  const [saveError, setSaveError] = useState<string | null>(null);
  const hydrated = useRef(false);
  const lastQueuedFingerprint = useRef("");
  const latestFingerprint = useRef("");
  const latestSnapshot = useRef<IntelligenceSnapshot | null>(null);
  const saveQueue = useRef<Promise<void>>(Promise.resolve());
  const savedStatusTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => { Promise.all([getContextPreferences(), getOutputPolicyConfig(), getSnippets(), getVocabularySuggestions()]).then(([c, p, s, l]) => {
    const snapshot = { context: c, policy: p, snippets: s };
    const initialFingerprint = fingerprint(snapshot);
    latestSnapshot.current = snapshot;
    latestFingerprint.current = initialFingerprint;
    lastQueuedFingerprint.current = initialFingerprint;
    setContext(c); setPolicy(p); setSnippetItems(s); setSuggestions(l); hydrated.current = true;
  }).catch((error) => { setSaveStatus("error"); setSaveError(`Não foi possível carregar as preferências: ${String(error)}`); }); }, []);

  const persistSnapshot = useCallback((snapshot: IntelligenceSnapshot, snapshotFingerprint: string) => {
    if (snapshotFingerprint === lastQueuedFingerprint.current) return;
    lastQueuedFingerprint.current = snapshotFingerprint;
    saveQueue.current = saveQueue.current.catch(() => undefined).then(async () => {
      if (latestFingerprint.current === snapshotFingerprint) setSaveStatus("saving");
      try {
        await setContextPreferences(snapshot.context);
        await setOutputPolicyConfig(snapshot.policy);
        await setSnippets(snapshot.snippets);
        if (latestFingerprint.current === snapshotFingerprint) {
          setSaveError(null); setSaveStatus("saved");
          if (savedStatusTimer.current) clearTimeout(savedStatusTimer.current);
          savedStatusTimer.current = setTimeout(() => setSaveStatus("idle"), 1600);
        }
      } catch (error) {
        if (latestFingerprint.current === snapshotFingerprint) {
          setSaveStatus("error");
          setSaveError(`Não foi possível salvar automaticamente: ${String(error)}`);
        }
      }
    });
  }, []);

  useEffect(() => {
    if (!hydrated.current || !context || !policy) return;
    const snapshot = { context, policy, snippets };
    const snapshotFingerprint = fingerprint(snapshot);
    latestSnapshot.current = snapshot;
    latestFingerprint.current = snapshotFingerprint;
    if (snapshotFingerprint === lastQueuedFingerprint.current) return;
    const validationError = validateSnapshot(snapshot);
    if (validationError) {
      setSaveStatus("invalid"); setSaveError(validationError);
      return;
    }
    setSaveStatus("pending"); setSaveError(null);
    const timer = setTimeout(() => persistSnapshot(snapshot, snapshotFingerprint), 500);
    return () => clearTimeout(timer);
  }, [context, policy, snippets, persistSnapshot]);

  useEffect(() => () => {
    if (savedStatusTimer.current) clearTimeout(savedStatusTimer.current);
    const snapshot = latestSnapshot.current;
    if (hydrated.current && snapshot && !validateSnapshot(snapshot) && latestFingerprint.current !== lastQueuedFingerprint.current) {
      persistSnapshot(snapshot, latestFingerprint.current);
    }
  }, [persistSnapshot]);
  if ((!context || !policy) && saveError) return <ErrorState><p>{saveError}</p><button type="button" className="mt-1 font-medium underline" onClick={() => window.location.reload()}>Recarregar preferências</button></ErrorState>;
  if (!context || !policy) return <p className="text-[13px] text-muted">Carregando preferências…</p>;
  const updateProfile = (index: number, change: Partial<OutputProfile>) => setPolicy({ ...policy, profiles: policy.profiles.map((profile, i) => i === index ? { ...profile, ...change } : profile) });
  const updateSnippet = (index: number, change: Partial<VoiceSnippet>) => setSnippetItems(snippets.map((item, i) => i === index ? { ...item, ...change } : item));

  return (
    <div>
      <Section title="Saída do ditado">
        <RowGroup>
          <PreferenceRow title="Formatação" description="Literal preserva a fala; Smart corrige casos claros; Aggressive reorganiza mais." htmlFor="formatting-level">
            <Select id="formatting-level" aria-label="Nível de formatação" className="w-[180px]" value={policy.formatting_level} onChange={(event) => setPolicy({ ...policy, formatting_level: event.target.value as OutputPolicyConfig["formatting_level"] })}>
              <option value="literal">Literal</option><option value="smart">Smart</option><option value="aggressive">Aggressive</option>
            </Select>
          </PreferenceRow>
          <PreferenceRow title="Destino" description="Salvar como nota guarda o texto em Histórico › Notas e nunca cola." htmlFor="output-destination">
            <Select id="output-destination" aria-label="Destino do ditado" className="w-[180px]" value={policy.destination} onChange={(event) => setPolicy({ ...policy, destination: event.target.value as OutputPolicyConfig["destination"] })}>
              <option value="focused_field">Colar no campo em foco</option><option value="clipboard_only">Só copiar</option><option value="scratchpad">Salvar como nota</option>
            </Select>
          </PreferenceRow>
          <PreferenceRow title="Style fixo" description="Ignora a detecção por aplicativo até você voltar para Automático." htmlFor="temporary-style">
            <Select id="temporary-style" aria-label="Style temporário" className="w-[180px]" value={policy.temporary_override ?? ""} onChange={(event) => setPolicy({ ...policy, temporary_override: event.target.value || null })}>
              <option value="">Automático</option>{policy.profiles.filter((profile) => profile.enabled).map((profile) => <option key={profile.id} value={profile.id}>{profile.name}</option>)}
            </Select>
          </PreferenceRow>
        </RowGroup>
      </Section>

      <Section
        title="Styles por aplicativo"
        description="Ajustam formatação e tom conforme onde você dita. Precedência: style fixo, domínio, aplicativo e padrão."
        action={<Button size="sm" onClick={() => setPolicy({ ...policy, profiles: [...policy.profiles, { id: `profile-${Date.now()}`, name: "Novo style", enabled: true, matcher: { processes: [], executables: [], window_titles: [], domains: [] }, formatting_level: null, content_type: null, style_instruction: null, allow_context_to_cloud: false }] })}><Plus className="h-3.5 w-3.5" aria-hidden />Novo style</Button>}
      >
        {policy.profiles.length === 0 ? <p className="border-y border-line py-4 text-[13px] text-muted">Nenhum style. Sem styles, todos os aplicativos usam a saída padrão.</p> : (
          <div className="hairline-list border-y border-line">
            {policy.profiles.map((profile, index) => {
              const where = [...profile.matcher.processes, ...profile.matcher.domains].slice(0, 3).join(", ");
              return (
                <details key={profile.id} className="group/p py-1">
                  <summary className="flex cursor-pointer items-center justify-between gap-4 py-3">
                    <span className="min-w-0">
                      <span className={"block text-[13px] font-medium " + (profile.enabled ? "text-ink" : "text-muted")}>{profile.name}{!profile.enabled && <span className="ml-2 font-normal">· pausado</span>}</span>
                      <span className="block truncate text-[12.5px] text-muted">{where || "Sem aplicativos ou domínios definidos"}</span>
                    </span>
                    <span className="text-[12.5px] text-muted group-hover/p:text-ink group-open/p:hidden">Editar</span>
                    <span className="hidden text-[12.5px] text-muted group-open/p:inline">Fechar</span>
                  </summary>
                  <div className="mb-4 mt-1 space-y-4">
                    <div className="grid grid-cols-2 gap-3 max-[720px]:grid-cols-1">
                      <label><span className="field-label">Nome</span><Input aria-label="Nome do style" value={profile.name} onChange={(event) => updateProfile(index, { name: event.target.value })} /></label>
                      <label><span className="field-label">ID</span><Input className="font-mono text-[12px]" value={profile.id} onChange={(event) => updateProfile(index, { id: event.target.value })} /></label>
                      <label><span className="field-label">Processos</span><Input aria-label="Processos do style" value={profile.matcher.processes.join(", ")} onChange={(event) => updateProfile(index, { matcher: { ...profile.matcher, processes: split(event.target.value) } })} placeholder="Code.exe, chrome.exe" /></label>
                      <label><span className="field-label">Domínios</span><Input aria-label="Domínios do style" value={profile.matcher.domains.join(", ")} onChange={(event) => updateProfile(index, { matcher: { ...profile.matcher, domains: split(event.target.value) } })} placeholder="chatgpt.com" /></label>
                      <label><span className="field-label">Títulos de janela contendo</span><Input aria-label="Títulos de janela do style" value={profile.matcher.window_titles.join(", ")} onChange={(event) => updateProfile(index, { matcher: { ...profile.matcher, window_titles: split(event.target.value) } })} placeholder="Codex, E-mail" /></label>
                      <label><span className="field-label">Executáveis</span><Input aria-label="Executáveis do style" value={profile.matcher.executables.join(", ")} onChange={(event) => updateProfile(index, { matcher: { ...profile.matcher, executables: split(event.target.value) } })} placeholder="C:\Apps\app.exe" /></label>
                      <label><span className="field-label">Formatação</span><Select aria-label="Formatação do style" wrapperClassName="w-full" className="w-full" value={profile.formatting_level ?? ""} onChange={(event) => updateProfile(index, { formatting_level: (event.target.value || null) as OutputProfile["formatting_level"] })}><option value="">Herdar</option><option value="literal">Literal</option><option value="smart">Smart</option><option value="aggressive">Aggressive</option></Select></label>
                      <label><span className="field-label">Conteúdo</span><Select aria-label="Tipo de conteúdo do style" wrapperClassName="w-full" className="w-full" value={profile.content_type ?? ""} onChange={(event) => updateProfile(index, { content_type: event.target.value || null })}><option value="">Automático</option><option value="programming">Programação</option><option value="study">Estudo</option></Select></label>
                    </div>
                    <label className="block"><span className="field-label">Instrução de estilo</span><Textarea aria-label="Instrução do style" className="min-h-20" value={profile.style_instruction ?? ""} onChange={(event) => updateProfile(index, { style_instruction: event.target.value || null })} placeholder="Ex.: frases curtas, sem emojis" /></label>
                    <div className="flex flex-wrap items-center justify-between gap-4">
                      <div className="flex flex-wrap items-center gap-6">
                        <span className="flex items-center gap-2.5 text-[12.5px] text-strong"><Toggle label={`Ativar ${profile.name}`} checked={profile.enabled} onChange={(enabled) => updateProfile(index, { enabled })} />Ativo</span>
                        <label className="flex items-center gap-2 text-[12.5px] text-strong"><input type="checkbox" checked={profile.allow_context_to_cloud ?? false} onChange={(event) => updateProfile(index, { allow_context_to_cloud: event.target.checked })} />Permitir contexto na nuvem neste style</label>
                      </div>
                      <Button size="sm" variant="danger" onClick={() => setPolicy({ ...policy, profiles: policy.profiles.filter((_, i) => i !== index) })}><Trash2 className="h-3.5 w-3.5" aria-hidden />Remover style</Button>
                    </div>
                  </div>
                </details>
              );
            })}
          </div>
        )}
      </Section>

      <Section
        title="Snippets por voz"
        description={<>Diga o gatilho sozinho, ou “snippet” seguido do gatilho no meio de uma frase, e o Sonora insere o texto completo. Ex.: “meu perfil é <span className="font-medium text-ink">snippet meu github</span>”.</>}
        action={<Button size="sm" onClick={() => setSnippetItems([...snippets, { id: `snippet-${Date.now()}`, trigger: "", expansion: "", enabled: true, require_activation_phrase: true }])}><Plus className="h-3.5 w-3.5" aria-hidden />Novo snippet</Button>}
      >
        {snippets.length === 0 ? <p className="border-y border-line py-4 text-[13px] text-muted">Nenhum snippet ainda.</p> : (
          <div className="hairline-list border-y border-line">
            {snippets.map((snippet, index) => (
              <div key={snippet.id} className="group py-4">
                <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.6fr)_auto] items-center gap-2 max-[720px]:grid-cols-1">
                  <Input aria-label="Trigger do snippet" value={snippet.trigger} onChange={(event) => updateSnippet(index, { trigger: event.target.value })} placeholder="Gatilho falado" />
                  <Input aria-label="Expansão do snippet" value={snippet.expansion} onChange={(event) => updateSnippet(index, { expansion: event.target.value })} placeholder="Texto inserido" />
                  <button type="button" aria-label="Remover snippet" title="Remover snippet" className="icon-button h-9 w-9 hover:text-live" onClick={() => setSnippetItems(snippets.filter((_, i) => i !== index))}><Trash2 className="h-4 w-4" aria-hidden /></button>
                </div>
                <div className="mt-2.5 flex flex-wrap items-center gap-5 text-[12.5px] text-muted">
                  <label className="flex items-center gap-2"><input type="checkbox" checked={snippet.enabled} onChange={(event) => updateSnippet(index, { enabled: event.target.checked })} />Ativo</label>
                  <label className="flex items-center gap-2"><input type="checkbox" checked={snippet.require_activation_phrase} onChange={(event) => updateSnippet(index, { require_activation_phrase: event.target.checked })} />Exigir “snippet” ou “expandir” antes do gatilho</label>
                </div>
              </div>
            ))}
          </div>
        )}
      </Section>

      <Section title="Aprender com suas correções" description="Quando você corrige a mesma palavra no Histórico duas vezes, o Sonora oferece guardá-la no vocabulário.">
        <RowGroup>
          <PreferenceRow title="Sugerir na hora" description="Mostra um aviso logo após a segunda correção. Desligado, as sugestões ficam só aqui.">
            <Toggle label="Sugerir vocabulário na hora" checked={features?.learning_prompts ?? true} disabled={!features} onChange={(learning_prompts) => void updateFeatures({ learning_prompts })} />
          </PreferenceRow>
        </RowGroup>
        {suggestions.length > 0 && (
          <div className="hairline-list mt-4 border-y border-line">
            {suggestions.map((event) => (
              <div key={event.id} className="flex items-center justify-between gap-5 py-3">
                <p className="text-[13px] text-ink"><span className="text-muted line-through decoration-line-strong">{event.before}</span> → <span className="font-medium">{event.after}</span><span className="timecode ml-2">{event.count}×</span></p>
                <div className="flex gap-1">
                  <Button size="sm" variant="ghost" onClick={() => void resolveVocabularySuggestion(event.id, false).then(() => setSuggestions((items) => items.filter((item) => item.id !== event.id))).catch((error) => { setSaveStatus("error"); setSaveError(String(error)); })}>Ignorar</Button>
                  <Button size="sm" onClick={() => void resolveVocabularySuggestion(event.id, true).then(() => setSuggestions((items) => items.filter((item) => item.id !== event.id))).catch((error) => { setSaveStatus("error"); setSaveError(String(error)); })}><Check className="h-3.5 w-3.5" aria-hidden />Adicionar</Button>
                </div>
              </div>
            ))}
          </div>
        )}
      </Section>

      <Section title="Contexto e privacidade" description="O Sonora pode usar o que está ao redor do cursor para acertar termos. Metadados ficam no computador; texto da tela só vai à nuvem com permissão geral e por fonte.">
        <RowGroup>
          <PreferenceRow title="Permitir contexto na nuvem" description="Ainda exige que cada fonte abaixo esteja como “Nuvem autorizada”.">
            <Toggle label="Permitir contexto na nuvem" checked={context.allow_context_to_cloud} onChange={(allow_context_to_cloud) => setContext({ ...context, allow_context_to_cloud })} />
          </PreferenceRow>
        </RowGroup>
        <details className="group/c mt-1">
          <summary className="inline-flex cursor-pointer items-center gap-1.5 py-3 text-[12.5px] font-medium text-muted hover:text-ink">
            <span className="inline-block h-1.5 w-1.5 -rotate-45 border-b-[1.5px] border-r-[1.5px] border-current transition-transform group-open/c:rotate-45" aria-hidden />
            Fontes de contexto e limites
          </summary>
          <RowGroup>
            {context.sources.map((source, index) => (
              <PreferenceRow key={source.source} title={sourceLabels[source.source]} description={!source.enabled ? "Desativada." : source.privacy === "cloud_allowed" ? "Pode ir à nuvem, delimitada e marcada como não confiável." : source.privacy === "ephemeral_local" ? "Usada só neste computador e descartada em seguida." : "Somente metadados."}>
                <Select aria-label={`Privacidade de ${sourceLabels[source.source]}`} className="w-[170px]" disabled={!source.enabled} value={source.privacy} onChange={(event) => setContext({ ...context, sources: context.sources.map((item, i) => i === index ? { ...item, privacy: event.target.value as typeof item.privacy } : item) })}>
                  <option value="metadata_only">Só metadados</option><option value="ephemeral_local">Local efêmero</option><option value="cloud_allowed">Nuvem autorizada</option>
                </Select>
                <Toggle label={`Ativar ${sourceLabels[source.source]}`} checked={source.enabled} onChange={(enabled) => setContext({ ...context, sources: context.sources.map((item, i) => i === index ? { ...item, enabled } : item) })} />
              </PreferenceRow>
            ))}
            <PreferenceRow title="Guardar contexto textual" description="Desligado por padrão. Aplicativo e domínio continuam nos detalhes técnicos.">
              <Toggle label="Persistir contexto textual" checked={context.persist_raw_context} onChange={(persist_raw_context) => setContext({ ...context, persist_raw_context })} />
            </PreferenceRow>
            <PreferenceRow title="Limite por fonte" description="Máximo de caracteres de seleção, cursor ou clipboard (100 a 4.000)." htmlFor="context-limit">
              <Input id="context-limit" aria-label="Limite de caracteres por fonte" className="w-24 text-right tabular-nums" type="number" min={100} max={4000} value={context.max_context_chars} onChange={(event) => setContext({ ...context, max_context_chars: Number(event.target.value) })} />
            </PreferenceRow>
          </RowGroup>
          <p className="mt-3 max-w-[64ch] text-[12px] leading-5 text-muted">No Chrome e Chromium, a extensão em <span className="font-mono">browser-extension</span> e o host em <span className="font-mono">native-messaging-host</span> enviam só domínio, URL sem query, seleção e até 800 caracteres próximos ao campo.</p>
        </details>
      </Section>

      <div className="pointer-events-none sticky bottom-5 mt-8 flex justify-end" aria-live="polite">
        {saveStatus !== "idle" && (
          <div className={`pointer-events-auto inline-flex h-8 items-center gap-2 rounded-full border px-3.5 text-[12px] shadow-menu ${saveStatus === "error" ? "border-live/30 bg-live-wash text-live" : saveStatus === "invalid" ? "border-standby/30 bg-standby-wash text-standby" : "border-line bg-raised text-muted"}`} title={saveError ?? undefined}>
            {saveStatus === "pending" || saveStatus === "saving" ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> : saveStatus === "error" || saveStatus === "invalid" ? <AlertCircle className="h-3.5 w-3.5" aria-hidden /> : <Check className="h-3.5 w-3.5 text-cue" aria-hidden />}
            {saveStatus === "pending" ? "Alterações pendentes" : saveStatus === "saving" ? "Salvando…" : saveStatus === "invalid" ? saveError ?? "Complete os campos para salvar" : saveStatus === "error" ? "Falha ao salvar" : "Salvo"}
          </div>
        )}
      </div>
    </div>
  );
}
