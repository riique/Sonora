import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Check, Copy, Trash2 } from "lucide-react";
import { deleteScratchpadNote, getScratchpadNotes, type ScratchpadNote } from "../lib/tauri";
import { Button } from "../components/ui/Button";
import { EmptyState, PageHeader, SkeletonRows } from "../components/ui/Surface";
import { clockTime, dayLabel } from "../lib/format";

/** Quick notes dictated with the "Salvar como nota" destination. Lives as a tab of Histórico. */
export function ScratchpadView({ embedded = false }: { embedded?: boolean }) {
  const [notes, setNotes] = useState<ScratchpadNote[]>([]);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState("");
  const [copied, setCopied] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    try {
      setNotes(await getScratchpadNotes());
      setMessage("");
    } catch (e) {
      setMessage(`Não foi possível carregar as notas: ${String(e)}`);
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => {
    void refresh();
    const pending = listen("transcription-saved", refresh);
    return () => { void pending.then((dispose) => dispose()); };
  }, [refresh]);
  const copy = async (id: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(id);
      window.setTimeout(() => setCopied((current) => (current === id ? null : current)), 1500);
    } catch {
      setMessage("Não foi possível copiar. Selecione e copie o texto da nota.");
    }
  };
  const remove = async (id: string) => {
    if (!window.confirm("Excluir esta nota?")) return;
    try {
      if (await deleteScratchpadNote(id)) setNotes((items) => items.filter((item) => item.id !== id));
    } catch (e) {
      setMessage(`Não foi possível excluir: ${String(e)}`);
    }
  };

  return (
    <div>
      {!embedded && <PageHeader title="Notas" description="Ditados salvos como nota, sem colar no aplicativo em foco." />}
      {message && (
        <div role="status" className="mb-5 flex flex-wrap items-center gap-3 text-[13px] text-live">
          <p>{message}</p>
          <Button size="sm" onClick={() => void refresh()}>Atualizar notas</Button>
        </div>
      )}
      {loading ? (
        <SkeletonRows count={3} />
      ) : notes.length === 0 ? (
        <div className="border-t border-line">
          <EmptyState title="Nenhuma nota rápida" description="Escolha “Salvar como nota” como destino do próximo ditado, na tela Início, e dite normalmente." />
        </div>
      ) : (
        <ul className="hairline-list border-y border-line">
          {notes.map((note) => {
            const date = new Date(note.created_at_ms);
            return (
              <li key={note.id} className="group grid grid-cols-[minmax(0,1fr)_auto] gap-5 py-4">
                <div className="min-w-0 max-w-[68ch]">
                  <time className="timecode">{dayLabel(date)} · {clockTime(date)}</time>
                  <p className="mt-1.5 wrap-break-word whitespace-pre-wrap text-[13.5px] leading-[1.6] text-ink">{note.text}</p>
                </div>
                <div className="flex items-start gap-0.5">
                  <button type="button" className={"icon-button " + (copied === note.id ? "text-cue" : "reveal-on-row")} onClick={() => void copy(note.id, note.text)} aria-label="Copiar nota" title="Copiar">
                    {copied === note.id ? <Check className="h-4 w-4" aria-hidden /> : <Copy className="h-4 w-4" aria-hidden />}
                  </button>
                  <button type="button" className="icon-button reveal-on-row hover:text-live" onClick={() => void remove(note.id)} aria-label="Excluir nota" title="Excluir">
                    <Trash2 className="h-4 w-4" aria-hidden />
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
