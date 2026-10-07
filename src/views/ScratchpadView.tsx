import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Clipboard, Trash2 } from "lucide-react";
import { deleteScratchpadNote, getScratchpadNotes, type ScratchpadNote } from "../lib/tauri";
import { Button } from "../components/ui/Button";
import { EmptyState, PageHeader } from "../components/ui/Surface";

export function ScratchpadView() {
  const [notes, setNotes] = useState<ScratchpadNote[]>([]);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState("");
  const refresh = useCallback(async () => { try { setNotes(await getScratchpadNotes()); } catch (e) { setMessage(`Não foi possível carregar as notas: ${String(e)}`); } finally { setLoading(false); } }, []);
  useEffect(() => { void refresh(); const pending = listen("transcription-saved", refresh); return () => { void pending.then((dispose) => dispose()); }; }, [refresh]);
  const copy = async (text: string) => { try { await navigator.clipboard.writeText(text); setMessage("Nota copiada."); } catch { setMessage("Não foi possível copiar. Selecione e copie o texto da nota."); } };
  const remove = async (id: string) => { if (!window.confirm("Excluir esta nota?")) return; try { if (await deleteScratchpadNote(id)) setNotes((items) => items.filter((item) => item.id !== id)); } catch (e) { setMessage(`Não foi possível excluir: ${String(e)}`); } };
  return (
    <div className="space-y-8">
      <PageHeader title="Scratchpad" description="Notas rápidas ditadas sem colar no aplicativo em foco." />
      {loading && <p role="status">Carregando notas…</p>}
      {message && <div role="status" className="text-sm"><p>{message}</p><Button size="sm" onClick={() => void refresh()}>Atualizar notas</Button></div>}
      <div className="overflow-hidden rounded-[12px] border border-line bg-white">
        {!loading && notes.length === 0 ? <EmptyState title="Nenhuma nota rápida" description="Escolha Scratchpad como destino e dite normalmente pelo gadget." /> : (
          <div className="divide-y divide-line">
            {notes.map((note) => (
              <article key={note.id} className="px-5 py-4">
                <div className="flex items-start justify-between gap-6">
                  <div className="min-w-0">
                    <time className="text-[11px] tabular-nums text-muted">{new Date(note.created_at_ms).toLocaleString("pt-BR")}</time>
                    <p className="mt-2 wrap-break-word whitespace-pre-wrap text-[14px] leading-6 text-ink">{note.text}</p>
                  </div>
                  <div className="flex shrink-0 gap-1">
                    <Button size="sm" variant="ghost" onClick={() => void copy(note.text)} aria-label="Copiar nota"><Clipboard className="h-4 w-4" aria-hidden /></Button>
                    <Button size="sm" variant="danger" onClick={() => void remove(note.id)} aria-label="Excluir nota"><Trash2 className="h-4 w-4" aria-hidden /></Button>
                  </div>
                </div>
              </article>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
