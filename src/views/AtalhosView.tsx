import { useEffect, useState } from "react";
import { Button } from "../components/ui/Button";
import { KbdCombo, shortcutKeys } from "../components/ui/Kbd";
import { ErrorState, PreferenceRow, RowGroup } from "../components/ui/Surface";
import { getShortcuts, setShortcuts, type ShortcutConfig } from "../lib/tauri";

type BindId = "toggle" | "cancel";

const BIND_META = [
  { id: "toggle" as const, title: "Iniciar e encerrar ditado", description: "Começa a gravar ou encerra a gravação ativa, em qualquer aplicativo." },
  { id: "cancel" as const, title: "Cancelar ditado", description: "Descarta a gravação em andamento sem salvar." },
];

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

/** Global shortcut capture rows, shown in Ajustes › Geral. */
export function ShortcutSettings() {
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
    <div>
      {error && <ErrorState>{error}</ErrorState>}
      <RowGroup>
        {BIND_META.map((binding) => {
          const isCapturing = capturing === binding.id;
          return (
            <PreferenceRow key={binding.id} title={binding.title} description={binding.description}>
              {isCapturing ? (
                <input
                  autoFocus
                  readOnly
                  aria-label={`Capturar novo atalho para ${binding.title}`}
                  onKeyDown={(event) => void handleKeyDown(binding.id, event)}
                  onBlur={() => setCapturing(null)}
                  value="Pressione as teclas…"
                  className="h-8 w-44 rounded-[8px] border border-ink bg-raised px-3 text-center text-[12px] text-ink outline-hidden"
                />
              ) : (
                <button
                  type="button"
                  disabled={saving}
                  onClick={() => { setError(""); setCapturing(binding.id); }}
                  className="group flex items-center gap-3 rounded-[8px] px-1 py-1 transition-colors"
                  aria-label={`Alterar atalho: ${binding.title}`}
                >
                  <KbdCombo keys={shortcutKeys(config[binding.id])} />
                  <span className="text-[12.5px] text-muted group-hover:text-ink">Alterar</span>
                </button>
              )}
              {isCapturing && <Button size="sm" variant="ghost" onMouseDown={(event) => event.preventDefault()} onClick={() => setCapturing(null)}>Cancelar</Button>}
            </PreferenceRow>
          );
        })}
      </RowGroup>
    </div>
  );
}
