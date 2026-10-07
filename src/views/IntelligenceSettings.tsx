import { useCallback, useEffect, useRef, useState } from "react";
import { AlertCircle, Check, Loader2, Plus, Trash2, X } from "lucide-react";
import {
  getContextPreferences, getOutputPolicyConfig, getSnippets, getVocabularySuggestions,
  resolveVocabularySuggestion, setContextPreferences, setOutputPolicyConfig, setSnippets,
  type ContextPreferences, type ContextSourceKind, type CorrectionEvent, type OutputPolicyConfig,
  type OutputProfile, type VoiceSnippet,
} from "../lib/tauri";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Toggle } from "../components/ui/Toggle";
import { PreferenceRow } from "../components/ui/Surface";

const selectClass = "h-9 rounded-[9px] border border-line bg-white px-3 text-[13px] text-ink outline-hidden focus:border-[#9b9c95]";
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
  if ((!context || !policy) && saveError) return <div role="alert" className="space-y-3 text-sm"><p>{saveError}</p><Button onClick={() => window.location.reload()}>Recarregar preferências</Button></div>;
  if (!context || !policy) return <p className="text-[13px] text-muted">Carregando preferências…</p>;
  const updateProfile = (index: number, change: Partial<OutputProfile>) => setPolicy({ ...policy, profiles: policy.profiles.map((profile, i) => i === index ? { ...profile, ...change } : profile) });

  return (
    <div className="space-y-10">
      <section>
        <h3 className="text-[15px] font-semibold text-ink">Saída do ditado</h3>
        <div className="mt-3 divide-y divide-line border-y border-line">
          <PreferenceRow title="Nível de formatação" description="Literal preserva a fala; Smart corrige casos claros; Aggressive permite reorganização maior.">
            <select aria-label="Nível de formatação" className={selectClass} value={policy.formatting_level} onChange={(event) => setPolicy({ ...policy, formatting_level: event.target.value as OutputPolicyConfig["formatting_level"] })}>
              <option value="literal">Literal</option><option value="smart">Smart</option><option value="aggressive">Aggressive</option>
            </select>
          </PreferenceRow>
          <PreferenceRow title="Destino" description="Scratchpad salva uma nota no Sonora e nunca cola no aplicativo em foco.">
            <select aria-label="Destino do ditado" className={selectClass} value={policy.destination} onChange={(event) => setPolicy({ ...policy, destination: event.target.value as OutputPolicyConfig["destination"] })}>
              <option value="focused_field">Campo em foco</option><option value="clipboard_only">Somente clipboard</option><option value="scratchpad">Scratchpad</option>
            </select>
          </PreferenceRow>
          <PreferenceRow title="Override temporário" description="Tem precedência sobre domínio e aplicativo até ser removido.">
            <select aria-label="Style temporário" className={selectClass} value={policy.temporary_override ?? ""} onChange={(event) => setPolicy({ ...policy, temporary_override: event.target.value || null })}>
              <option value="">Automático</option>{policy.profiles.filter((profile) => profile.enabled).map((profile) => <option key={profile.id} value={profile.id}>{profile.name}</option>)}
            </select>
          </PreferenceRow>
        </div>
      </section>

      <section>
        <h3 className="text-[15px] font-semibold text-ink">Contexto e privacidade</h3>
        <p className="mt-1 text-[13px] leading-5 text-muted">Metadados são locais. Texto da tela só vai à nuvem com opt-in global e por fonte.</p>
        <div className="mt-3 divide-y divide-line border-y border-line">
          {context.sources.map((source, index) => (
            <PreferenceRow key={source.source} title={sourceLabels[source.source]} description={source.privacy === "cloud_allowed" ? "Pode ser enviado como contexto não confiável e delimitado." : "Permanece local e efêmero."}>
              <div className="flex items-center gap-3">
                <select aria-label={`Privacidade de ${sourceLabels[source.source]}`} className={selectClass} disabled={!source.enabled} value={source.privacy} onChange={(event) => setContext({ ...context, sources: context.sources.map((item, i) => i === index ? { ...item, privacy: event.target.value as typeof item.privacy } : item) })}>
                  <option value="metadata_only">Só metadados</option><option value="ephemeral_local">Local efêmero</option><option value="cloud_allowed">Nuvem autorizada</option>
                </select>
                <Toggle label={`Ativar ${sourceLabels[source.source]}`} checked={source.enabled} onChange={(enabled) => setContext({ ...context, sources: context.sources.map((item, i) => i === index ? { ...item, enabled } : item) })} />
              </div>
            </PreferenceRow>
          ))}
          <PreferenceRow title="Permitir contexto na nuvem" description="Ainda exige que cada fonte esteja marcada como Nuvem autorizada."><Toggle label="Permitir contexto na nuvem" checked={context.allow_context_to_cloud} onChange={(allow_context_to_cloud) => setContext({ ...context, allow_context_to_cloud })} /></PreferenceRow>
          <PreferenceRow title="Persistir contexto textual" description="Desligado por padrão. Metadados de aplicativo/domínio continuam disponíveis no Inspector."><Toggle label="Persistir contexto textual" checked={context.persist_raw_context} onChange={(persist_raw_context) => setContext({ ...context, persist_raw_context })} /></PreferenceRow>
          <PreferenceRow title="Limite por fonte" description="Quantidade máxima de caracteres de seleção, cursor ou clipboard."><Input aria-label="Limite de caracteres por fonte" className="w-24" type="number" min={100} max={4000} value={context.max_context_chars} onChange={(event) => setContext({ ...context, max_context_chars: Number(event.target.value) })} /></PreferenceRow>
        </div>
        <details className="mt-3 text-[12px] text-muted"><summary className="cursor-pointer font-medium text-[#555650]">Integração com Chrome/Chromium</summary><p className="mt-2 max-w-[76ch] leading-5">A extensão mínima e o Native Messaging Host estão em browser-extension e native-messaging-host. Eles enviam somente domínio, URL sem query, seleção e até 800 caracteres próximos ao campo.</p></details>
      </section>

      <section>
        <div className="flex items-center justify-between"><div><h3 className="text-[15px] font-semibold text-ink">Styles por aplicativo</h3><p className="mt-1 text-[13px] text-muted">Precedência: override temporário, domínio, aplicativo e padrão.</p></div><Button size="sm" onClick={() => setPolicy({ ...policy, profiles: [...policy.profiles, { id: `profile-${Date.now()}`, name: "Novo style", enabled: true, matcher: { processes: [], executables: [], window_titles: [], domains: [] }, formatting_level: null, content_type: null, style_instruction: null, allow_context_to_cloud: false }] })}><Plus className="h-4 w-4" />Adicionar</Button></div>
        <div className="mt-4 space-y-3">
          {policy.profiles.map((profile, index) => (
            <details key={profile.id} className="rounded-[10px] border border-line bg-white px-4 py-3" open={index === 0}>
              <summary className="cursor-pointer text-[13px] font-medium text-ink">{profile.name}<span className="ml-2 text-[11px] font-normal text-muted">{profile.id}</span></summary>
              <div className="mt-4 grid grid-cols-2 gap-3 max-[900px]:grid-cols-1">
                <Input aria-label="Nome do style" value={profile.name} onChange={(event) => updateProfile(index, { name: event.target.value })} placeholder="Nome" />
                <Input value={profile.id} onChange={(event) => updateProfile(index, { id: event.target.value })} placeholder="ID" />
                <Input aria-label="Processos do style" value={profile.matcher.processes.join(", ")} onChange={(event) => updateProfile(index, { matcher: { ...profile.matcher, processes: split(event.target.value) } })} placeholder="Processos: Code.exe, chrome.exe" />
                <Input aria-label="Executáveis do style" value={profile.matcher.executables.join(", ")} onChange={(event) => updateProfile(index, { matcher: { ...profile.matcher, executables: split(event.target.value) } })} placeholder="Executáveis: C:\\Apps\\app.exe" />
                <Input aria-label="Títulos de janela do style" value={profile.matcher.window_titles.join(", ")} onChange={(event) => updateProfile(index, { matcher: { ...profile.matcher, window_titles: split(event.target.value) } })} placeholder="Títulos contendo: Codex, E-mail" />
                <Input aria-label="Domínios do style" value={profile.matcher.domains.join(", ")} onChange={(event) => updateProfile(index, { matcher: { ...profile.matcher, domains: split(event.target.value) } })} placeholder="Domínios: chatgpt.com" />
                <select aria-label="Formatação do style" className={selectClass} value={profile.formatting_level ?? ""} onChange={(event) => updateProfile(index, { formatting_level: (event.target.value || null) as OutputProfile["formatting_level"] })}><option value="">Herdar formatação</option><option value="literal">Literal</option><option value="smart">Smart</option><option value="aggressive">Aggressive</option></select>
                <select aria-label="Tipo de conteúdo do style" className={selectClass} value={profile.content_type ?? ""} onChange={(event) => updateProfile(index, { content_type: event.target.value || null })}><option value="">Conteúdo automático</option><option value="programming">Programação</option><option value="study">Estudo</option></select>
                <Input aria-label="Instrução do style" value={profile.style_instruction ?? ""} onChange={(event) => updateProfile(index, { style_instruction: event.target.value || null })} placeholder="Instrução de estilo" />
              </div>
              <div className="mt-3 flex items-center justify-between gap-4"><div className="flex items-center gap-5"><Toggle label={`Ativar ${profile.name}`} checked={profile.enabled} onChange={(enabled) => updateProfile(index, { enabled })} /><label className="flex items-center gap-2 text-[12px] text-muted"><input type="checkbox" checked={profile.allow_context_to_cloud ?? false} onChange={(event) => updateProfile(index, { allow_context_to_cloud: event.target.checked })} />Permitir contexto cloud neste style</label></div><Button size="sm" variant="danger" onClick={() => setPolicy({ ...policy, profiles: policy.profiles.filter((_, i) => i !== index) })}><Trash2 className="h-4 w-4" />Remover</Button></div>
            </details>
          ))}
        </div>
      </section>

      <section>
        <div className="flex items-center justify-between"><div><h3 className="text-[15px] font-semibold text-ink">Snippets por voz</h3><p className="mt-1 text-[13px] text-muted">Matching exato, local e aplicado depois dos modelos.</p></div><Button size="sm" onClick={() => setSnippetItems([...snippets, { id: `snippet-${Date.now()}`, trigger: "", expansion: "", enabled: true, require_activation_phrase: true }])}><Plus className="h-4 w-4" />Adicionar</Button></div>
        <div className="mt-4 space-y-3">{snippets.map((snippet, index) => <div key={snippet.id} className="rounded-[10px] border border-line bg-white p-4"><div className="grid grid-cols-[1fr_1.5fr_auto] gap-3 max-[900px]:grid-cols-1"><Input aria-label="Trigger do snippet" value={snippet.trigger} onChange={(event) => setSnippetItems(snippets.map((item, i) => i === index ? { ...item, trigger: event.target.value } : item))} placeholder="Trigger falado" /><Input aria-label="Expansão do snippet" value={snippet.expansion} onChange={(event) => setSnippetItems(snippets.map((item, i) => i === index ? { ...item, expansion: event.target.value } : item))} placeholder="Expansão literal" /><Button aria-label="Remover snippet" size="sm" variant="danger" onClick={() => setSnippetItems(snippets.filter((_, i) => i !== index))}><Trash2 className="h-4 w-4" /></Button></div><div className="mt-3 flex items-center gap-5 text-[12px] text-muted"><label className="flex items-center gap-2"><input type="checkbox" checked={snippet.enabled} onChange={(event) => setSnippetItems(snippets.map((item, i) => i === index ? { ...item, enabled: event.target.checked } : item))} />Ativo</label><label className="flex items-center gap-2"><input type="checkbox" checked={snippet.require_activation_phrase} onChange={(event) => setSnippetItems(snippets.map((item, i) => i === index ? { ...item, require_activation_phrase: event.target.checked } : item))} />Exigir “snippet” ou “expandir”</label></div></div>)}</div>
      </section>

      <section>
        <h3 className="text-[15px] font-semibold text-ink">Sugestões de vocabulário</h3>
        <p className="mt-1 text-[13px] text-muted">Somente correções lexicais repetidas três vezes aparecem aqui.</p>
        <div className="mt-3 divide-y divide-line border-y border-line">{suggestions.length === 0 ? <p className="py-5 text-[13px] text-muted">Nenhuma sugestão pendente.</p> : suggestions.map((event) => <div key={event.id} className="flex items-center justify-between gap-5 py-4"><p className="text-[13px] text-ink">“{event.before}” → <span className="font-medium">“{event.after}”</span><span className="ml-2 text-muted">{event.count}×</span></p><div className="flex gap-1"><Button size="sm" onClick={() => void resolveVocabularySuggestion(event.id, true).then(() => setSuggestions((items) => items.filter((item) => item.id !== event.id))).catch((error) => { setSaveStatus("error"); setSaveError(String(error)); })}><Check className="h-4 w-4" />Adicionar</Button><Button size="sm" variant="ghost" onClick={() => void resolveVocabularySuggestion(event.id, false).then(() => setSuggestions((items) => items.filter((item) => item.id !== event.id))).catch((error) => { setSaveStatus("error"); setSaveError(String(error)); })}><X className="h-4 w-4" />Ignorar</Button></div></div>)}</div>
      </section>

      <div className="sticky bottom-4 flex justify-end" aria-live="polite">
        {saveStatus !== "idle" && (
          <div className={`inline-flex min-h-9 items-center gap-2 rounded-[9px] border px-3 text-[12px] shadow-xs ${saveStatus === "error" ? "border-red-200 bg-red-50 text-red-700" : saveStatus === "invalid" ? "border-amber-200 bg-amber-50 text-amber-800" : "border-line bg-white text-muted"}`} title={saveError ?? undefined}>
            {saveStatus === "pending" || saveStatus === "saving" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : saveStatus === "error" || saveStatus === "invalid" ? <AlertCircle className="h-3.5 w-3.5" /> : <Check className="h-3.5 w-3.5" />}
            {saveStatus === "pending" ? "Aguardando alterações…" : saveStatus === "saving" ? "Salvando automaticamente…" : saveStatus === "invalid" ? "Complete os campos para salvar" : saveStatus === "error" ? "Falha no salvamento automático" : "Alterações salvas"}
          </div>
        )}
      </div>
    </div>
  );
}
