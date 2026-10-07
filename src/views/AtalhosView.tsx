import { useEffect, useState } from "react";
import { Ban, Mic2 } from "lucide-react";
import { Button } from "../components/ui/Button";
import { KbdCombo } from "../components/ui/Kbd";
import { ErrorState, PageHeader } from "../components/ui/Surface";
import { getShortcuts, setShortcuts, type ShortcutConfig } from "../lib/tauri";

type BindId = "toggle" | "cancel";

const BIND_META = [
  { id: "toggle" as const, title: "Iniciar / parar ditado", description: "Inicia uma nova gravação ou encerra a gravação ativa.", icon: Mic2 },
  { id: "cancel" as const, title: "Cancelar ditado", description: "Descarta a gravação em andamento sem salvar uma transcrição.", icon: Ban },
];

function toKeys(shortcut: string): string[] {
  const labels: Record<string, string> = { Control: "Ctrl", CommandOrControl: "Ctrl", Alt: "Alt", Shift: "Shift", Super: "Win", Meta: "Win" };
  return shortcut.split("+").map((key) => labels[key] ?? key);
}

function eventToShortcut(event: React.KeyboardEvent): string | null {
  const modifiers: string[] = [];
  if (event.ctrlKey) modifiers.push("Control");
  if (event.altKey) modifiers.push("Alt");
  if (event.shiftKey) modifiers.push("Shift");
  if (event.metaKey) modifiers.push("Super");
  const key = event.key;
  if (["Control", "Alt", "Shift", "Meta", "Dead"].includes(key)) return null;
  let main = "";
  if (/^[a-zA-Z]$/.test(key)) main = key.toUpperCase();
  else if (/^[0-9]$/.test(key)) main = key;
  else if (/^F([1-9]|1[0-2])$/.test(key)) main = key;
  else main = ({ " ": "Space", ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right", Enter: "Enter", Tab: "Tab", Backspace: "Backspace", ",": "Comma", ".": "Period", "/": "Slash", ";": "Semicolon" } as Record<string, string>)[key] ?? "";
  if (!main || (modifiers.length === 0 && !/^F([1-9]|1[0-2])$/.test(main))) return null;
  return [...modifiers, main].join("+");
}

export function AtalhosView() {
  const [config, setConfig] = useState<ShortcutConfig>({ toggle: "Control+B", cancel: "Control+Q" });
  const [capturing, setCapturing] = useState<BindId | null>(null);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    getShortcuts().then(setConfig).catch((loadError) => console.error("get_shortcuts failed:", loadError));
  }, []);

  const handleKeyDown = async (bind: BindId, event: React.KeyboardEvent<HTMLInputElement>) => {
    event.preventDefault();
    if (event.key === "Escape") {
      setCapturing(null);
      return;
    }
    const combo = eventToShortcut(event);
    if (!combo) {
      setError("Use ao menos um modificador com a tecla, ou uma tecla de função.");
      return;
    }
    const next = bind === "toggle" ? { toggle: combo, cancel: config.cancel } : { toggle: config.toggle, cancel: combo };
    setSaving(true);
    setError("");
    try {
      setConfig(await setShortcuts(next.toggle, next.cancel));
      setCapturing(null);
    } catch (saveError) {
      setError(typeof saveError === "string" ? saveError : String(saveError));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-8">
      <PageHeader title="Atalhos" description="Personalize os atalhos globais que controlam o Sonora em qualquer aplicativo." />
      {error && <ErrorState>{error}</ErrorState>}
      <section aria-labelledby="dictation-shortcuts">
        <h2 id="dictation-shortcuts" className="section-title border-b border-line pb-3">Ditado</h2>
        <div className="divide-y divide-line">
          {BIND_META.map((binding) => {
            const Icon = binding.icon;
            const isCapturing = capturing === binding.id;
            return (
              <div key={binding.id} className="grid min-h-[92px] grid-cols-[44px_minmax(0,1fr)_auto_auto] items-center gap-5 py-4 max-[850px]:grid-cols-[44px_minmax(0,1fr)_auto]">
                <span className="flex h-9 w-9 items-center justify-center rounded-full border border-line bg-white text-[#555650]">
                  <Icon className="h-4 w-4" aria-hidden />
                </span>
                <div className="min-w-0">
                  <h3 className="text-[14px] font-medium text-ink">{binding.title}</h3>
                  <p className="mt-1 text-[13px] leading-5 text-muted">{binding.description}</p>
                </div>
                <div className="min-w-36 justify-self-end max-[850px]:col-start-2 max-[850px]:row-start-2 max-[850px]:justify-self-start">
                  {isCapturing ? (
                    <input
                      autoFocus
                      readOnly
                      aria-label={`Capturar novo atalho para ${binding.title}`}
                      onKeyDown={(event) => void handleKeyDown(binding.id, event)}
                      onBlur={() => setCapturing(null)}
                      value="Pressione as teclas…"
                      className="h-9 w-44 rounded-[8px] border border-[#8f9089] bg-white px-3 text-center text-[12px] text-ink outline-hidden"
                    />
                  ) : <KbdCombo keys={toKeys(config[binding.id])} />}
                </div>
                <Button
                  size="sm"
                  disabled={saving}
                  className="max-[850px]:col-start-3 max-[850px]:row-start-1"
                  onClick={() => {
                    setError("");
                    setCapturing(isCapturing ? null : binding.id);
                  }}
                >
                  {isCapturing ? "Cancelar" : "Alterar"}
                </Button>
              </div>
            );
          })}
        </div>
        <p className="border-t border-line pt-4 text-[12px] text-muted">Os atalhos funcionam globalmente enquanto o Sonora estiver em execução. Pressione Esc para cancelar uma captura.</p>
      </section>
    </div>
  );
}
