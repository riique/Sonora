import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/Surface";
import { cancelRecording, getHistoryPage, type HistoryEntry } from "../lib/tauri";

interface Diagnostics {
  version: string; microphone: string | null; microphone_available: boolean;
  missing_providers: string[]; operation: { id: number; kind: string; cancelled: boolean } | null;
  storage_errors: string[]; recovery_audio: { id: string; bytes: number }[];
}
export function RecoveryView() {
  const [diagnostics, setDiagnostics] = useState<Diagnostics | null>(null);
  const [deleted, setDeleted] = useState<HistoryEntry[]>([]);
  const [offset, setOffset] = useState(0);
  const [totalDeleted, setTotalDeleted] = useState(0);
  const [includeAudio, setIncludeAudio] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const refresh = useCallback(async () => {
    try {
      const status = await invoke<Diagnostics>("get_local_diagnostics"); setDiagnostics(status);
      if (!status.storage_errors.length) { const page = await getHistoryPage("", offset, 20, true); setDeleted(page.items); setTotalDeleted(page.total); }
    } catch (e) { setMessage(String(e)); }
  }, [offset]);
  useEffect(() => { void refresh(); const events = ["transcription-saved", "storage-error", "capture-error"].map((name) => listen(name, refresh)); return () => { events.forEach((pending) => void pending.then((dispose) => dispose())); }; }, [refresh]);
  const act = async (work: () => Promise<unknown>, success: string) => {
    if (busy) return; setBusy(true); setMessage("");
    try { await work(); setMessage(success); await refresh(); } catch (e) { setMessage(String(e)); } finally { setBusy(false); }
  };
  const exportData = async () => {
    const destination = await save({ defaultPath: `sonora-backup-${new Date().toISOString().slice(0, 10)}.json`, filters: [{ name: "Backup Sonora", extensions: ["json"] }] });
    if (destination) await act(() => invoke("export_local_data", { destination, includeAudio }), "Backup exportado. Guarde-o em um local privado.");
  };
  const archiveAudio = async (id: string) => {
    const destination = await open({ directory: true, multiple: false });
    if (typeof destination !== "string" || !window.confirm("Arquivar o áudio nesta pasta? A cópia será verificada antes de remover o original. O texto permanecerá recuperável e o áudio continuará acessível enquanto a pasta estiver disponível.")) return;
    await act(async () => { const result = await invoke<string>("archive_history_audio", { id, destination }); setMessage(result); }, "Áudio arquivado; confira a pasta selecionada.");
  };
  const importData = async () => {
    const source = await open({ multiple: false, filters: [{ name: "Backup Sonora", extensions: ["json"] }] });
    if (typeof source === "string" && window.confirm("Importar este backup? Histórico, notas, vocabulário, snippets e styles serão mesclados. Configurações ativas serão preservadas e um backup será criado antes da importação.")) {
      await act(() => invoke("import_local_data", { source }), "Importação concluída. Confira os dados importados.");
    }
  };
  return <div className="space-y-8">
    <PageHeader title="Diagnóstico e recuperação" description="Confira a configuração local e recupere ditados sem gravar novamente." action={<Button disabled={busy} onClick={() => void refresh()}>Atualizar</Button>} />
    {message && <p role="status" className="wrap-break-word rounded-lg border border-line p-4 text-sm">{message}</p>}
    {!diagnostics ? <p role="status">Carregando diagnóstico…</p> : <>
      <section aria-labelledby="readiness-title" className="space-y-3 border-y border-line py-5">
        <h2 id="readiness-title" className="section-title">Prontidão local</h2>
        <p className="text-sm">Microfone: {diagnostics.microphone ?? "Padrão do Windows"} · {diagnostics.microphone_available ? "dispositivo disponível" : "dispositivo indisponível"}.</p>
        <p className="text-sm">{diagnostics.missing_providers.length ? `Configure as chaves de: ${diagnostics.missing_providers.join(", ")}.` : "As credenciais necessárias à rota principal estão configuradas."}</p>
        <p className="text-sm text-muted">Disponibilidade física, permissões e resposta dos provedores precisam ser verificadas durante o uso. Este diagnóstico não grava áudio nem chama modelos.</p>
        {diagnostics.storage_errors.map((error) => <p role="alert" key={error} className="text-sm text-[#9f2720]">{error}</p>)}
        {diagnostics.storage_errors.length > 0 && <Button disabled={busy} onClick={() => { if (window.confirm("Preservar uma cópia do histórico e remover somente a última transação incompleta?")) void act(() => invoke("repair_history_journal"), "Histórico reparado; cópia original preservada."); }}>Reparar histórico interrompido</Button>}
        {diagnostics.operation && !["import", "export", "archive", "history-edit", "voice-profile"].includes(diagnostics.operation.kind) && <div className="flex flex-wrap items-center gap-3"><p role="status" className="text-sm">Operação ativa: {diagnostics.operation.kind}</p><Button onClick={() => void act(cancelRecording, "Cancelamento solicitado. O áudio de recuperação será preservado.")}>Cancelar operação</Button></div>}
      </section>
      <section aria-labelledby="audio-recovery-title" className="space-y-3">
        <h2 id="audio-recovery-title" className="section-title">Áudios interrompidos</h2>
        <p className="text-sm text-muted">Retranscrever usa a pipeline configurada e envia o áudio ao provedor selecionado. Cada gravação permite até 15 minutos.</p>
        {!diagnostics.recovery_audio.length && <p className="text-sm">Nenhum áudio aguardando recuperação.</p>}
        <div className="divide-y divide-line">{diagnostics.recovery_audio.map((audio) => <div key={audio.id} className="flex flex-wrap items-center justify-between gap-3 py-3"><span className="min-w-0 break-all text-sm">{audio.id} · {(audio.bytes / 1048576).toFixed(1)} MiB</span><Button size="sm" disabled={busy || !!diagnostics.operation} onClick={() => void act(() => invoke("retry_recovery_audio", { id: audio.id }), "Áudio recuperado no histórico.")}>Retranscrever áudio</Button></div>)}</div>
      </section>
      <section aria-labelledby="deleted-title" className="space-y-3">
        <h2 id="deleted-title" className="section-title">Itens removidos</h2>
        <p className="text-sm text-muted">O histórico preserva o texto e o áudio dos itens removidos. Para liberar espaço na pasta atual, arquive o áudio em outra pasta. Não há limpeza automática.</p>
        {deleted.map((entry) => <article key={entry.id} className="flex items-start justify-between gap-4 border-b border-line py-3"><p className="min-w-0 wrap-break-word text-sm">{entry.text.slice(0, 200) || entry.error_message || "Ditado sem texto"}</p><div className="flex flex-wrap gap-2">{entry.audio_path && <Button size="sm" disabled={busy || !!diagnostics?.operation} onClick={() => void archiveAudio(entry.id).catch((error) => setMessage(String(error)))}>Arquivar áudio</Button>}<Button size="sm" disabled={busy} onClick={() => void act(() => invoke("restore_history_entry", { id: entry.id }), "Item restaurado no histórico.")}>Restaurar</Button></div></article>)}
        {!totalDeleted && <p className="text-sm">Nenhum item removido.</p>}
        {totalDeleted > 20 && <nav aria-label="Paginação dos itens removidos" className="flex gap-3"><Button disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - 20))}>Anterior</Button><Button disabled={offset + 20 >= totalDeleted} onClick={() => setOffset(offset + 20)}>Próxima</Button></nav>}
      </section>
    </>}
    <section aria-labelledby="backup-title" className="space-y-3 border-t border-line pt-5">
      <h2 id="backup-title" className="section-title">Backup dos dados</h2>
      <p className="max-w-prose text-sm text-muted">Exporta textos, vocabulário, snippets e preferências. As chaves de API ficam fora deste arquivo. Se incluir áudios, guarde também a pasta .media ao lado do JSON. O backup contém conteúdo pessoal.</p>
      <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={includeAudio} onChange={(e) => setIncludeAudio(e.target.checked)} />Incluir áudios associados ao histórico</label>
      <div className="flex flex-wrap gap-3"><Button disabled={busy || !!diagnostics?.operation} onClick={() => void exportData().catch((e) => setMessage(String(e)))}>Exportar dados</Button><Button disabled={busy || !!diagnostics?.operation} onClick={() => void importData().catch((e) => setMessage(String(e)))}>Importar backup</Button></div>
    </section>
  </div>;
}
