import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Button } from "../components/ui/Button";

interface RecoveryAudio { id: string; bytes: number }

/**
 * Gravações que não chegaram ao histórico (o app fechou ou caiu no meio).
 * Só aparece no Histórico quando existe algo para recuperar.
 */
export function InterruptedAudio({ onRecovered }: { onRecovered?: () => void }) {
  const [items, setItems] = useState<RecoveryAudio[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState("");

  const refresh = useCallback(() => {
    invoke<RecoveryAudio[]>("list_recovery_audio").then(setItems).catch(() => setItems([]));
  }, []);

  useEffect(() => {
    refresh();
    const pending = ["transcription-saved", "capture-error"].map((name) => listen(name, refresh));
    return () => { pending.forEach((p) => void p.then((dispose) => dispose())); };
  }, [refresh]);

  if (!items.length) return null;

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

  return (
    <section className="mb-8 border-y border-line py-4" aria-labelledby="interrupted-audio-title">
      <h2 id="interrupted-audio-title" className="text-[13px] font-medium text-ink">
        {items.length === 1 ? "1 gravação interrompida" : `${items.length} gravações interrompidas`}
      </h2>
      <p className="mt-0.5 text-[12.5px] text-muted">O áudio foi guardado, mas não chegou a virar texto. Retranscrever envia o áudio ao provedor.</p>
      <ul className="mt-3 space-y-2">
        {items.map((audio) => (
          <li key={audio.id} className="flex flex-wrap items-center justify-between gap-3">
            <span className="min-w-0 break-all font-mono text-[12px] text-strong">{audio.id} <span className="text-muted">· {(audio.bytes / 1048576).toFixed(1)} MiB</span></span>
            <Button size="sm" disabled={busy !== null} onClick={() => void retry(audio.id)}>{busy === audio.id ? "Retranscrevendo…" : "Retranscrever"}</Button>
          </li>
        ))}
      </ul>
      {message && <p role="status" className="mt-3 text-[12.5px] text-muted">{message}</p>}
    </section>
  );
}
