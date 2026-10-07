import { useEffect, useState } from "react";
import { Button } from "../components/ui/Button";
import { KbdCombo, shortcutKeys } from "../components/ui/Kbd";
import { ErrorState, PreferenceRow, RowGroup, Segmented } from "../components/ui/Surface";
import { getShortcuts, setShortcuts, type ShortcutConfig } from "../lib/tauri";

type BindId = "toggle" | "cancel" | "command";
type RecordStyle = "toggle" | "hold";

const BIND_META: { id: BindId; title: string; description: (hold: boolean) => string }[] = [
  {
    id: "toggle",
    title: "Ditar",
    description: (hold) => hold ? "Segure enquanto fala; ao soltar, o texto é colado." : "Aperte para começar e de novo para colar o texto.",
  },
  {
    id: "command",
    title: "Comando de voz",
    description: (hold) =>
      `Selecione um texto, ${hold ? "segure" : "aperte"} e diga o que fazer: “deixa mais formal”, “traduz para inglês”. Sem seleção, escreve o que você pedir.`,
  },
  { id: "cancel", title: "Cancelar", description: () => "Descarta a gravação em andamento sem colar nada." },
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
  const [config, setConfig] = useState<ShortcutConfig>({ toggle: "Control+B", cancel: "Control+Q", command: "Control+Shift+B", hold_to_talk: false });
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
    await save({ ...config, [bind]: combo });
  };

  const save = async (next: ShortcutConfig) => {
    setSaving(true);
    setError("");
    try {
      setConfig(await setShortcuts(next));
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
        <PreferenceRow title="Modo de gravação" description="Vale para ditar e para o comando de voz.">
          <Segmented<RecordStyle>
            label="Modo de gravação"
            value={config.hold_to_talk ? "hold" : "toggle"}
            onChange={(style) => void save({ ...config, hold_to_talk: style === "hold" })}
            options={[{ value: "toggle", label: "Apertar" }, { value: "hold", label: "Segurar" }]}
          />
        </PreferenceRow>
        {BIND_META.map((binding) => {
          const isCapturing = capturing === binding.id;
          const combo = config[binding.id];
          return (
            <PreferenceRow key={binding.id} title={binding.title} description={binding.description(config.hold_to_talk)}>
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
              ) : combo ? (
                <button
                  type="button"
                  disabled={saving}
                  onClick={() => { setError(""); setCapturing(binding.id); }}
                  className="group flex items-center gap-3 rounded-[8px] px-1 py-1 transition-colors"
                  aria-label={`Alterar atalho: ${binding.title}`}
                >
                  <KbdCombo keys={shortcutKeys(combo)} />
                  <span className="text-[12.5px] text-muted group-hover:text-ink">Alterar</span>
                </button>
              ) : (
                <Button size="sm" disabled={saving} onClick={() => { setError(""); setCapturing(binding.id); }}>
                  Definir atalho
                </Button>
              )}
              {isCapturing && <Button size="sm" variant="ghost" onMouseDown={(event) => event.preventDefault()} onClick={() => setCapturing(null)}>Cancelar</Button>}
              {!isCapturing && binding.id === "command" && combo && (
                <Button size="sm" variant="ghost" disabled={saving} onClick={() => void save({ ...config, command: "" })}>
                  Desativar
                </Button>
              )}
            </PreferenceRow>
          );
        })}
      </RowGroup>
    </div>
  );
}
