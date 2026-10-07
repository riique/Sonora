import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Check, X } from "lucide-react";
import { Button } from "../components/ui/Button";
import { Section } from "../components/ui/Surface";
import { cancelRecording, getHistoryPage, type HistoryEntry } from "../lib/tauri";
import { shortStamp } from "../lib/format";

interface Diagnostics {
  version: string; microphone: string | null; microphone_available: boolean;
  missing_providers: string[]; operation: { id: number; kind: string; cancelled: boolean } | null;
  storage_errors: string[]; recovery_audio: { id: string; bytes: number }[];
}

function CheckRow({ ok, children }: { ok: boolean; children: React.ReactNode }) {
  return (
    <li className="flex items-start gap-3 py-3">
      <span className={"mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full " + (ok ? "bg-cue-wash text-cue" : "bg-standby-wash text-standby")} aria-hidden>
        {ok ? <Check className="h-3 w-3" strokeWidth={2.5} /> : <X className="h-3 w-3" strokeWidth={2.5} />}
      </span>
      <span className="text-[13px] leading-5 text-ink">{children}</span>
    </li>
  );
}

/** Diagnóstico, áudios interrompidos, itens removidos e backup — Ajustes › Dados e recuperação. */
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
  const operationActive = !!diagnostics?.operation;

  return <div>
    {message && <p role="status" className="mb-8 wrap-break-word border-y border-line py-3 text-[13px] text-ink">{message}</p>}

    <Section title="Verificação local" description="Confere a configuração sem gravar áudio nem chamar modelos." action={<Button size="sm" variant="ghost" disabled={busy} onClick={() => void refresh()}>Verificar de novo</Button>}>
      {!diagnostics ? <p role="status" className="py-3 text-[13px] text-muted">Verificando…</p> : <>
        <ul className="hairline-list border-y border-line">
          <CheckRow ok={diagnostics.microphone_available}>Microfone: {diagnostics.microphone ?? "Padrão do Windows"} · {diagnostics.microphone_available ? "disponível" : "indisponível"}</CheckRow>
          <CheckRow ok={!diagnostics.missing_providers.length}>{diagnostics.missing_providers.length ? `Faltam chaves de: ${diagnostics.missing_providers.join(", ")}` : "Chaves da rota principal configuradas"}</CheckRow>
          <CheckRow ok={!diagnostics.storage_errors.length}>{diagnostics.storage_errors.length ? "Histórico com transação interrompida" : "Histórico íntegro"}</CheckRow>
        </ul>
        {diagnostics.storage_errors.map((error) => <p role="alert" key={error} className="mt-3 text-[12.5px] text-live">{error}</p>)}
        {diagnostics.storage_errors.length > 0 && <Button className="mt-3" size="sm" disabled={busy} onClick={() => { if (window.confirm("Preservar uma cópia do histórico e remover somente a última transação incompleta?")) void act(() => invoke("repair_history_journal"), "Histórico reparado; cópia original preservada."); }}>Reparar histórico interrompido</Button>}
        {diagnostics.operation && !["import", "export", "archive", "history-edit", "voice-profile"].includes(diagnostics.operation.kind) && <div className="mt-3 flex flex-wrap items-center gap-3"><p role="status" className="text-[13px]">Operação ativa: {diagnostics.operation.kind}</p><Button size="sm" onClick={() => void act(cancelRecording, "Cancelamento solicitado. O áudio de recuperação será preservado.")}>Cancelar operação</Button></div>}
      </>}
    </Section>

    {diagnostics && <>
      <Section title="Áudios interrompidos" description="Gravações que não chegaram ao histórico. Retranscrever usa o modo ativo e envia o áudio ao provedor; até 15 minutos por gravação.">
        {!diagnostics.recovery_audio.length ? <p className="border-y border-line py-4 text-[13px] text-muted">Nenhum áudio aguardando recuperação.</p> : (
          <ul className="hairline-list border-y border-line">{diagnostics.recovery_audio.map((audio) => <li key={audio.id} className="flex flex-wrap items-center justify-between gap-3 py-3"><span className="min-w-0 break-all font-mono text-[12px] text-strong">{audio.id} <span className="text-muted">· {(audio.bytes / 1048576).toFixed(1)} MiB</span></span><Button size="sm" disabled={busy || operationActive} onClick={() => void act(() => invoke("retry_recovery_audio", { id: audio.id }), "Áudio recuperado no histórico.")}>Retranscrever</Button></li>)}</ul>
        )}
      </Section>

      <Section title="Itens removidos" description="O texto e o áudio dos itens removidos continuam guardados e podem ser restaurados. Não há limpeza automática; para liberar espaço, arquive o áudio em outra pasta.">
        {!totalDeleted ? <p className="border-y border-line py-4 text-[13px] text-muted">Nenhum item removido.</p> : (
          <ul className="hairline-list border-y border-line">{deleted.map((entry) => <li key={entry.id} className="group grid grid-cols-[52px_minmax(0,1fr)_auto] items-start gap-4 py-3"><span className="timecode pt-px">{shortStamp(entry.date)}</span><p className="min-w-0 line-clamp-2 text-[13px] leading-5 text-muted line-through decoration-line-strong">{entry.text.slice(0, 200) || entry.error_message || "Ditado sem texto"}</p><div className="flex gap-1">{entry.audio_path && <Button size="sm" variant="ghost" disabled={busy || operationActive} onClick={() => void archiveAudio(entry.id).catch((error) => setMessage(String(error)))}>Arquivar áudio</Button>}<Button size="sm" disabled={busy} onClick={() => void act(() => invoke("restore_history_entry", { id: entry.id }), "Item restaurado no histórico.")}>Restaurar</Button></div></li>)}</ul>
        )}
        {totalDeleted > 20 && <nav aria-label="Paginação dos itens removidos" className="mt-4 flex justify-end gap-2"><Button size="sm" variant="ghost" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - 20))}>Anteriores</Button><Button size="sm" disabled={offset + 20 >= totalDeleted} onClick={() => setOffset(offset + 20)}>Mais antigos</Button></nav>}
      </Section>
    </>}

    <Section title="Backup" description="Exporta textos, notas, vocabulário, snippets e preferências. As chaves de API ficam fora do arquivo. O backup contém conteúdo pessoal; guarde-o em local privado.">
      <div className="flex flex-wrap items-center justify-between gap-4 border-y border-line py-4">
        <label className="flex items-center gap-2.5 text-[13px] text-ink"><input type="checkbox" checked={includeAudio} onChange={(e) => setIncludeAudio(e.target.checked)} />Incluir áudios <span className="text-muted">(guarde a pasta .media ao lado do JSON)</span></label>
        <div className="flex gap-2"><Button size="sm" variant="ghost" disabled={busy || operationActive} onClick={() => void importData().catch((e) => setMessage(String(e)))}>Importar backup</Button><Button size="sm" disabled={busy || operationActive} onClick={() => void exportData().catch((e) => setMessage(String(e)))}>Exportar dados</Button></div>
      </div>
    </Section>
  </div>;
}
