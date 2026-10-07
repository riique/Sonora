import { useEffect, useState } from "react";
import { BookPlus, RefreshCw, X } from "lucide-react";
import { Button } from "./ui/Button";
import {
  installUpdate,
  onUpdateReady,
  onVocabularySuggestion,
  resolveVocabularySuggestion,
  type CorrectionEvent,
  type UpdateInfo,
} from "../lib/tauri";

type Notice =
  | { kind: "update"; id: string; info: UpdateInfo }
  | { kind: "vocabulary"; id: string; event: CorrectionEvent };

/**
 * Quiet, non-modal notices in the main window: a downloaded update and a
 * repeated correction that can become vocabulary. They wait for the user and
 * never steal focus.
 */
export function Notices() {
  const [notices, setNotices] = useState<Notice[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<{ id: string; message: string } | null>(null);

  useEffect(() => {
    const subscriptions = [
      onUpdateReady((info) =>
        setNotices((current) => [
          ...current.filter((notice) => notice.kind !== "update"),
          { kind: "update", id: `update-${info.available ?? "ready"}`, info },
        ]),
      ),
      onVocabularySuggestion((event) =>
        setNotices((current) =>
          current.some((notice) => notice.id === event.id)
            ? current
            : [...current, { kind: "vocabulary", id: event.id, event }],
        ),
      ),
    ];
    return () => {
      subscriptions.forEach((subscription) => subscription.then((unlisten) => unlisten()).catch(() => {}));
    };
  }, []);

  const dismiss = (id: string) => setNotices((current) => current.filter((notice) => notice.id !== id));

  const run = async (id: string, action: () => Promise<void>) => {
    setBusy(id);
    setError(null);
    try {
      await action();
      dismiss(id);
    } catch (actionError) {
      setError({ id, message: String(actionError) });
    } finally {
      setBusy(null);
    }
  };

  if (notices.length === 0) return null;

  return (
    <div className="notice-stack" role="region" aria-label="Avisos" aria-live="polite">
      {notices.map((notice) => (
        <article key={notice.id} className="notice animate-fade-in">
          <span className="notice-icon" aria-hidden>
            {notice.kind === "update" ? <RefreshCw className="h-3.5 w-3.5" /> : <BookPlus className="h-3.5 w-3.5" />}
          </span>
          <div className="min-w-0 flex-1">
            {notice.kind === "update" ? (
              <>
                <h2 className="text-[13px] font-medium text-ink">
                  Sonora {notice.info.available} está pronta
                </h2>
                <p className="mt-0.5 text-[12.5px] leading-5 text-muted">
                  Reinicie para atualizar agora ou deixe para a próxima vez que abrir o app.
                </p>
              </>
            ) : (
              <>
                <h2 className="text-[13px] font-medium text-ink">
                  Adicionar <span className="font-semibold">{notice.event.after}</span> ao vocabulário?
                </h2>
                <p className="mt-0.5 text-[12.5px] leading-5 text-muted">
                  Você trocou “{notice.event.before}” por “{notice.event.after}” {notice.event.count} vezes. O Sonora passa a
                  escrever assim sozinho.
                </p>
              </>
            )}
            {error?.id === notice.id && <p className="mt-1.5 text-[12px] text-live" role="alert">{error.message}</p>}
            <div className="mt-3 flex gap-2">
              {notice.kind === "update" ? (
                <Button size="sm" variant="primary" disabled={busy === notice.id} onClick={() => run(notice.id, installUpdate)}>
                  {busy === notice.id ? "Reiniciando…" : "Reiniciar agora"}
                </Button>
              ) : (
                <>
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={busy === notice.id}
                    onClick={() => run(notice.id, () => resolveVocabularySuggestion(notice.event.id, true))}
                  >
                    Adicionar
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy === notice.id}
                    onClick={() => run(notice.id, () => resolveVocabularySuggestion(notice.event.id, false))}
                  >
                    Não sugerir de novo
                  </Button>
                </>
              )}
            </div>
          </div>
          <button type="button" className="notice-close" aria-label="Fechar aviso" onClick={() => dismiss(notice.id)}>
            <X className="h-3.5 w-3.5" aria-hidden />
          </button>
        </article>
      ))}
    </div>
  );
}
