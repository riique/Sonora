import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Button } from "../components/ui/Button";

export interface RecoveryAudio { id: string; bytes: number }

/** Interrupted recordings (the app closed or crashed mid-dictation). */
export function useRecoveryAudio() {
  const [items, setItems] = useState<RecoveryAudio[]>([]);

  const refresh = useCallback(() => {
    invoke<RecoveryAudio[]>("list_recovery_audio").then(setItems).catch(() => setItems([]));
  }, []);

  useEffect(() => {
    refresh();
    const pending = ["transcription-saved", "capture-error"].map((name) => listen(name, refresh));
    return () => { pending.forEach((p) => void p.then((dispose) => dispose())); };
  }, [refresh]);

  return { items, refresh };
}

/**
 * Gravações que não chegaram ao histórico. Abre só a partir do menu do
 * Histórico, para não empurrar a lista de ditados para baixo.
 */
export function InterruptedAudio({
  items,
  refresh,
  onClose,
  onRecovered,
}: {
  items: RecoveryAudio[];
  refresh: () => void;
  onClose: () => void;
  onRecovered?: () => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState("");

  const retry = async (id: string) => {
    if (busy) return;
    setBusy(id);
    setMessage("");
    try {
      await invoke("retry_recovery_audio", { id });
      setMessage("Áudio recuperado no histórico.");
      onRecovered?.();
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(null);
      refresh();
    }
  };

  const dismissAll = async () => {
    if (busy) return;
    setBusy("__all__");
    setMessage("");
    try {
      await invoke<number>("dismiss_all_recovery_audio");
      onClose();
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(null);
      refresh();
    }
  };

  return (
    <section className="mb-8 animate-fade-in border-y border-line py-4" aria-labelledby="interrupted-audio-title">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 id="interrupted-audio-title" className="text-[13px] font-medium text-ink">
            {items.length === 1 ? "1 gravação interrompida" : `${items.length} gravações interrompidas`}
          </h2>
          <p className="mt-0.5 text-[12.5px] text-muted">
            O áudio foi guardado, mas não chegou a virar texto. Retranscrever envia o áudio ao provedor; dispensar só tira da lista, sem apagar o áudio.
          </p>
        </div>
        <div className="flex gap-1.5">
          {items.length > 0 && (
            <Button size="sm" variant="secondary" disabled={busy !== null} onClick={() => void dismissAll()}>
              {busy === "__all__" ? "Dispensando…" : "Dispensar todas"}
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={onClose}>Fechar</Button>
        </div>
      </div>
      {items.length > 0 ? (
        <ul className="mt-3 max-h-80 space-y-2 overflow-y-auto pr-1">
          {items.map((audio) => (
            <li key={audio.id} className="flex flex-wrap items-center justify-between gap-3">
              <span className="min-w-0 break-all font-mono text-[12px] text-strong">{audio.id} <span className="text-muted">· {(audio.bytes / 1048576).toFixed(1)} MiB</span></span>
              <Button size="sm" disabled={busy !== null} onClick={() => void retry(audio.id)}>{busy === audio.id ? "Retranscrevendo…" : "Retranscrever"}</Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-3 text-[12.5px] text-muted">Nenhuma gravação pendente.</p>
      )}
      {message && <p role="status" className="mt-3 text-[12.5px] text-muted">{message}</p>}
    </section>
  );
}
