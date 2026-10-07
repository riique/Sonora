import { useEffect, useRef, useState } from "react";
import { AlertCircle, Check, FileAudio, Loader2, Upload } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Button } from "../components/ui/Button";
import { cancelRecording, transcribeFile } from "../lib/tauri";

type Status = "idle" | "transcribing" | "done" | "error";

const AUDIO_EXTENSIONS = ["wav", "mp3", "m4a", "flac", "ogg", "aac", "webm", "mp4"];

function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

/** File transcription panel, opened from Histórico. Uses the active pipeline and never pastes. */
export function TranscricaoView() {
  const busy = useRef(false);
  const [filePath, setFilePath] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [status, setStatus] = useState<Status>("idle");
  const [result, setResult] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let mounted = true;
    void getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "over" || payload.type === "enter") {
        setDragging(true);
        return;
      }
      if (payload.type === "leave") {
        setDragging(false);
        return;
      }
      if (payload.type === "drop") {
        setDragging(false);
        const dropped = payload.paths?.[0];
        const extension = dropped?.split(".").pop()?.toLowerCase() ?? "";
        if (dropped && AUDIO_EXTENSIONS.includes(extension)) selectPath(dropped);
      }
    }).then((dispose) => {
      if (!mounted) dispose(); else unlisten = dispose;
    }).catch((failure) => { if (mounted) setError(String(failure)); });
    return () => { mounted = false; unlisten?.(); };
  }, []);

  const selectPath = (path: string) => {
    if (busy.current) return;
    setFilePath(path);
    setStatus("idle");
    setResult("");
    setError("");
  };

  const handleBrowse = async () => {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Áudio", extensions: AUDIO_EXTENSIONS }],
      });
      if (typeof selected === "string") selectPath(selected);
    } catch (browseError) {
      setError(String(browseError));
      setStatus("error");
    }
  };

  const handleTranscribe = async () => {
    if (!filePath || busy.current) return;
    busy.current = true;
    setStatus("transcribing");
    setError("");
    setResult("");
    try {
      setResult(await transcribeFile(filePath));
      setStatus("done");
    } catch (transcriptionError) {
      setError(typeof transcriptionError === "string" ? transcriptionError : String(transcriptionError));
      setStatus("error");
    } finally { busy.current = false; }
  };

  const transcribing = status === "transcribing";

  return (
    <section aria-labelledby="upload-title" className="space-y-4">
      <button
        type="button"
        disabled={transcribing}
        onClick={handleBrowse}
        className={
          "flex min-h-[148px] w-full flex-col items-center justify-center rounded-[12px] border border-dashed px-8 py-8 text-center transition-colors duration-150 disabled:cursor-not-allowed " +
          (dragging ? "border-ink bg-fill" : "border-line-strong bg-raised hover:border-faint hover:bg-fill/40")
        }
        aria-describedby="upload-formats"
      >
        {filePath ? <FileAudio className="h-5 w-5 text-ink" aria-hidden /> : <Upload className="h-5 w-5 text-muted" aria-hidden />}
        <span id="upload-title" className="mt-3 text-[13.5px] font-medium text-ink">
          {filePath ? baseName(filePath) : "Arraste um arquivo de áudio ou clique para escolher"}
        </span>
        <span id="upload-formats" className="mt-1 text-[12px] text-muted">
          {filePath ? "Clique para escolher outro arquivo" : "WAV, MP3, M4A, FLAC, OGG, AAC, WEBM ou MP4 · usa o modo de transcrição ativo"}
        </span>
      </button>

      <div className="flex flex-wrap items-center justify-end gap-2">
        {transcribing && <Button variant="ghost" onClick={() => void cancelRecording().catch((e) => setError(String(e)))}>Cancelar transcrição</Button>}
        <Button variant="primary" disabled={!filePath || transcribing} onClick={handleTranscribe}>
          {transcribing && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
          {transcribing ? "Transcrevendo…" : "Transcrever arquivo"}
        </Button>
      </div>

      {status === "error" && (
        <div className="flex items-start gap-2.5 rounded-[9px] bg-live-wash px-4 py-3 text-[13px] text-live" role="alert">
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
          <span className="min-w-0 wrap-break-word">{error}</span>
        </div>
      )}

      {status === "done" && (
        <div className="border-y border-line py-5">
          <p className="flex items-center gap-2 text-[12.5px] font-medium text-cue" role="status">
            <Check className="h-4 w-4" aria-hidden />
            Transcrição concluída e salva no histórico
          </p>
          <p className="mt-3 max-w-[68ch] whitespace-pre-wrap text-[13.5px] leading-[1.6] text-ink">{result}</p>
        </div>
      )}
    </section>
  );
}
