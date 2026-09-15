import { useEffect, useRef, useState } from "react";
import { Check, Loader2, RefreshCw } from "lucide-react";
import { Button } from "../components/ui/Button";
import { Toggle } from "../components/ui/Toggle";
import { ErrorState } from "../components/ui/Surface";
import {
  addInsightCorrectionToVocabulary, generateAiVoiceProfile, setAiVoiceProfileEnabled,
  type InsightsResponse,
} from "../lib/tauri";
import { formatInsightNumber as number, voiceProfileRemainingWords } from "./insights-utils";

function shortPortrait(text: string) {
  if (text.length <= 320) return text;
  const sentences = text.match(/[^.!?]+[.!?]+(?:\s|$)/g);
  const first = sentences?.slice(0, 2).join("").trim();
  return first && first.length <= 360 ? first : `${text.slice(0, 300).replace(/\s+\S*$/, "")}…`;
}

export function VoiceInsights({ data, reload, developerMode }: {
  data: InsightsResponse; reload: () => Promise<void>; developerMode: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const [vocabBusy, setVocabBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    if (!confirm) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const element = dialog.current;
    element?.showModal();
    element?.querySelector<HTMLButtonElement>("button")?.focus();
    return () => { element?.close(); previous?.focus(); };
  }, [confirm]);

  const toggleProfile = async (enabled: boolean) => {
    setSaving(true); setError(null);
    try { await setAiVoiceProfileEnabled(enabled); await reload(); }
    catch { setError("Não foi possível salvar a preferência. Tente novamente."); }
    finally { setSaving(false); }
  };
  const generate = async () => {
    setConfirm(false); setBusy(true); setError(null);
    try { await generateAiVoiceProfile(); await reload(); }
    catch { setError("Não foi possível criar seu retrato. Confira a chave do OpenRouter em Configurações e tente novamente."); }
    finally { setBusy(false); }
  };
  const addVocabulary = async () => {
    const correction = data.language.most_corrected;
    if (!correction) return;
    setVocabBusy(true); setError(null);
    try { await addInsightCorrectionToVocabulary(correction.before, correction.after); await reload(); }
    catch { setError("Não foi possível adicionar a palavra. Tente novamente."); }
    finally { setVocabBusy(false); }
  };

  const profile = data.profile;
  const evidence = data.voice_evidence;
  const portrait = profile?.personal_portrait?.summary || profile?.archetype.description || profile?.description || "";
  const excerpt = shortPortrait(portrait);
  const title = profile?.archetype.title || profile?.title || "Seu jeito de falar vai aparecer aqui";
  const signature = profile && Object.values(profile.signature).some(Boolean) ? profile.signature : evidence.signature_candidates;
  const phrase = signature.catchphrase || signature.phrase || data.language.most_used_phrase?.label;
  const topics = (profile?.recurring_topics.length ? profile.recurring_topics : evidence.recurring_topics).slice(0, 3);
  const patterns = [
    ...(profile?.personal_portrait?.distinctive_habits ?? []),
    ...(profile?.communication_patterns.length ? profile.communication_patterns : evidence.linguistic_patterns),
  ].filter((item, index, all) => item.confidence >= .4 && all.findIndex((other) => other.title === item.title) === index).slice(0, 3);
  const experiment = profile?.suggested_experiments?.find((item) => item.confidence >= .4);
  const correction = data.language.most_corrected;
  const remaining = voiceProfileRemainingWords(data.profile_progress_words, data.profile_required_words);
  const audio = data.audio;

  return <div className="voice-insights">
    <section className="voice-portrait" aria-labelledby="voice-portrait-title">
      <h2 id="voice-portrait-title">{title}</h2>
      <p className="voice-portrait__text">{excerpt || (data.profile_generation_ready
        ? "Você já tem ditados suficientes para criar um pequeno retrato do seu jeito de se expressar."
        : "Continue usando o ditado no seu dia a dia. Aos poucos, você vai descobrir as expressões e os hábitos que se repetem na sua fala.")}</p>
      {portrait !== excerpt && <details className="voice-full-portrait"><summary>Ler retrato completo</summary><p>{portrait}</p></details>}
      <div className="voice-portrait__action">
        {data.profile_enabled ? <>
          <Button variant="primary" disabled={busy || !data.profile_generation_ready} aria-describedby="voice-profile-progress" onClick={() => setConfirm(true)}>
            {busy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <RefreshCw className="h-4 w-4" aria-hidden />}
            {busy ? "Criando seu retrato…" : profile ? "Atualizar meu retrato" : "Criar meu retrato"}
          </Button>
          <p id="voice-profile-progress" className="text-[13px] leading-5 text-muted" aria-live="polite">
            {busy ? "Seu novo retrato aparecerá aqui quando estiver pronto."
              : data.profile_generation_ready ? (profile ? "Você já pode atualizar seu retrato." : "Você já pode criar seu primeiro retrato.")
              : `${remaining === 1 ? "Falta 1 palavra ditada" : `Faltam ${number(remaining)} palavras ditadas`} para ${profile ? "atualizar seu retrato" : "criar seu primeiro retrato"}.`}
          </p>
        </> : <p className="text-[13px] text-muted">Suas descobertas locais continuam aqui. O retrato com IA é opcional.</p>}
      </div>
    </section>

    {error && <ErrorState>{error}</ErrorState>}

    {(phrase || topics.length > 0) && <section className="voice-signature" aria-label="Suas expressões e seus assuntos">
      {phrase && <div><h3>Uma expressão que é sua</h3><blockquote>“{phrase}”</blockquote></div>}
      {topics.length > 0 && <div><h3>Assuntos que voltam à conversa</h3><ul className="voice-topics">{topics.map((topic) => <li key={topic.title}>{topic.title}</li>)}</ul></div>}
    </section>}

    {patterns.length > 0 && <section className="voice-habits" aria-labelledby="voice-habits-title">
      <h3 id="voice-habits-title">Pequenos hábitos da sua fala</h3>
      <div>{patterns.map((item) => <article key={item.title}><h4>{item.title}</h4><p>{item.description}</p></article>)}</div>
    </section>}
    {!phrase && !topics.length && !patterns.length && <p className="voice-learning">Ainda estamos conhecendo sua fala. Suas primeiras descobertas aparecem aqui conforme você dita.</p>}

    {experiment && <section className="voice-experiment"><h3>Para experimentar no próximo ditado</h3><h4>{experiment.title}</h4><p>{experiment.description}</p></section>}

    <details className="voice-details">
      <summary>Preferências e detalhes</summary>
      <div className="voice-details__body">
        <div className="voice-preference"><div><h3>Retrato com IA</h3><p>Opcional. Usa um resumo dos seus padrões de ditado para escrever seu retrato.</p></div><Toggle checked={data.profile_enabled} disabled={saving || busy} onChange={toggleProfile} label="Ativar retrato com IA" /></div>
        <p className="text-[12px] leading-5 text-muted">Você confirma antes de cada geração. O envio inclui estatísticas e termos filtrados, sem gravações nem transcrições completas.</p>
        <section><h3>Suas palavras</h3><dl className="voice-facts">
          <Fact label="Palavra frequente" value={signature.content_word || data.language.most_used_content_word?.label} />
          <Fact label="Como você costuma começar" value={signature.opener} />
          <Fact label="Expressão para conectar ideias" value={signature.connector} />
          <Fact label="Variedade de palavras" value={data.language.vocabulary_variety_label} />
        </dl></section>
        {correction && <section><h3>Uma correção que se repete</h3><p className="mt-3 text-[14px]"><span className="text-muted line-through">{correction.before}</span> → <strong>{correction.after}</strong></p>
          {correction.in_vocabulary ? <p className="mt-3 inline-flex items-center gap-1 text-[13px] text-[#25613f]"><Check className="h-4 w-4" aria-hidden /> Já está no vocabulário</p>
            : <Button className="mt-3" disabled={vocabBusy} onClick={addVocabulary}>{vocabBusy ? "Adicionando…" : "Adicionar ao vocabulário"}</Button>}
        </section>}
        <section><h3>Seu microfone</h3><p className="mt-3 text-[13px] leading-6 text-muted">{!audio.analyzed_sessions
          ? "Use o ditado pelo microfone para conhecer melhor sua captura de voz."
          : (audio.clipping_ratio ?? 0) > .01 ? "Algumas gravações têm picos de volume. Tente afastar um pouco o microfone."
          : audio.estimated_snr_db != null && audio.estimated_snr_db < 12 ? "Há ruído de fundo nas gravações. Um lugar mais silencioso pode ajudar."
          : audio.lufs_median != null && audio.lufs_median < -28 ? "Sua voz chega baixinha ao microfone. Se o ditado funcionar bem, não é preciso mudar nada."
          : "Continue usando a posição do microfone que funciona melhor para você."}</p>
          {audio.analyzed_sessions > 0 && <details className="mt-4"><summary>Medições de áudio</summary><dl className="voice-facts">
            <Fact label="Volume estimado" value={audio.lufs_median == null ? null : `${number(audio.lufs_median, 1)} LUFS`} />
            <Fact label="Nível médio (RMS)" value={audio.rms_dbfs_median == null ? null : `${number(audio.rms_dbfs_median, 1)} dBFS`} />
            <Fact label="Picos" value={audio.peak_dbfs_median == null ? null : `${number(audio.peak_dbfs_median, 1)} dBFS`} />
            <Fact label="Sinal e ruído" value={audio.estimated_snr_db == null ? null : `${number(audio.estimated_snr_db, 1)} dB`} />
            <Fact label="Frequência da voz" value={audio.median_f0_hz == null ? null : `${number(audio.median_f0_hz)} Hz`} />
            <Fact label="Pausas" value={audio.average_pause_ms == null ? null : `${number(audio.average_pause_ms)} ms`} />
          </dl></details>}
        </section>
        {data.language.fillers.length > 0 && <section><h3>Palavras de apoio</h3><p className="mt-2 text-[13px] text-muted">Expressões que acompanham sua fala. Não são erros.</p><dl className="voice-facts">{data.language.fillers.map((item) => <Fact key={item.phrase} label={`“${item.phrase}”`} value={`${number(item.per_1000_words, 1)} a cada mil palavras`} />)}</dl></section>}
        {developerMode && <details><summary>Diagnóstico técnico</summary><pre className="mt-4 max-h-80 overflow-auto whitespace-pre-wrap break-words text-[11px]">{JSON.stringify({ audio, evidence, profile, generation: data.profile_generation }, null, 2)}</pre></details>}
      </div>
    </details>

    {confirm && <dialog ref={dialog} className="voice-confirm" aria-labelledby="voice-confirm-title" aria-describedby="voice-confirm-description" onCancel={() => setConfirm(false)} onClose={() => setConfirm(false)}>
      <h2 id="voice-confirm-title">Criar seu retrato com IA?</h2>
      <p id="voice-confirm-description">O Sonora envia estatísticas e termos filtrados dos seus ditados. Suas gravações e transcrições completas ficam no computador.</p>
      <p>A geração usa sua chave do OpenRouter e pode ter custo. Modelo: <span className="break-words font-mono text-[12px]">meta/muse-spark-1.3-contributor</span>.</p>
      <div className="flex flex-wrap justify-end gap-2"><Button onClick={() => setConfirm(false)}>Cancelar</Button><Button variant="primary" onClick={generate}>Criar retrato</Button></div>
    </dialog>}
  </div>;
}

function Fact({ label, value }: { label: string; value?: string | null }) {
  return <div><dt>{label}</dt><dd>{value || "Ainda aprendendo"}</dd></div>;
}
