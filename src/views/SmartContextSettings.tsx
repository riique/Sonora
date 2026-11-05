import { useEffect, useState } from "react";
import { Check } from "lucide-react";
import { getSmartContextConfig, setSmartContextConfig, type SmartContextConfig } from "../lib/tauri";
import { Input, Select } from "../components/ui/Input";
import { Toggle } from "../components/ui/Toggle";
import { PreferenceRow, RowGroup, Section } from "../components/ui/Surface";

const DEFAULT: SmartContextConfig = {
  enabled: false,
  model: "nex-agi/nex-n2.5-mini",
  history_count: 5,
};

/** Impeccable Operate: restrained, progressive disclosure, explicit data consent. */
export function SmartContextSettings() {
  const [config, setConfig] = useState<SmartContextConfig>(DEFAULT);
  const [modelInput, setModelInput] = useState(DEFAULT.model);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    getSmartContextConfig()
      .then((saved) => { setConfig(saved); setModelInput(saved.model); })
      .catch((error) => setMessage({ ok: false, text: String(error) }))
      .finally(() => setLoading(false));
  }, []);

  const persist = async (next: SmartContextConfig) => {
    setSaving(true);
    setMessage(null);
    try {
      const saved = await setSmartContextConfig(next);
      setConfig(saved);
      setModelInput(saved.model);
      setMessage({ ok: true, text: "Salvo" });
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
    } finally {
      setSaving(false);
    }
  };

  return (
    <Section
      title="Contexto inteligente"
      description="Opcional e exclusivo da AssemblyAI. A transcrição continua mesmo se a análise da tela falhar."
      action={message && (
        <span role={message.ok ? "status" : "alert"} className={"inline-flex items-center gap-1.5 text-[12px] " + (message.ok ? "text-cue" : "text-live")}>
          {message.ok && <Check className="h-3.5 w-3.5" aria-hidden />}
          {message.text}
        </span>
      )}
    >
      <RowGroup>
        <PreferenceRow
          title="Analisar a janela ativa"
          description="Ao iniciar um ditado, envia uma imagem da janela ativa e trechos do histórico ao modelo de visão da OpenRouter para gerar contexto."
        >
          <Toggle
            label="Ativar contexto inteligente"
            checked={config.enabled}
            disabled={loading || saving}
            onChange={(enabled) => void persist({ ...config, enabled })}
          />
        </PreferenceRow>
        {config.enabled && (
          <>
            <PreferenceRow title="Modelo de visão" description="O modelo precisa aceitar imagens. A alteração é salva ao sair do campo." htmlFor="smart-context-model">
              <Input
                id="smart-context-model"
                className="w-[260px] max-w-full font-mono text-[12px]"
                value={modelInput}
                spellCheck={false}
                disabled={saving}
                onChange={(event) => setModelInput(event.target.value)}
                onBlur={() => { if (modelInput.trim() !== config.model) void persist({ ...config, model: modelInput.trim() }); }}
                onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }}
                placeholder="nex-agi/nex-n2.5-mini"
              />
            </PreferenceRow>
            <PreferenceRow title="Ditados anteriores" description="O histórico é enviado na ordem original, sem tradução." htmlFor="smart-context-history-count">
              <Select
                id="smart-context-history-count"
                className="w-[160px]"
                value={config.history_count}
                disabled={saving}
                onChange={(event) => void persist({ ...config, history_count: Number(event.target.value) as 5 | 10 })}
              >
                <option value={5}>Últimos 5</option>
                <option value={10}>Últimos 10</option>
              </Select>
            </PreferenceRow>
          </>
        )}
      </RowGroup>
      <p className="mt-3 max-w-[64ch] text-[12px] leading-5 text-muted">
        Desativado por padrão. Quando ativo, a captura e os ditados anteriores são enviados à OpenRouter;
        nenhuma imagem é salva no disco. Não ative em janelas com senhas ou dados confidenciais.
        O idioma é detectado automaticamente, inclusive com alternância entre português e inglês.
      </p>
    </Section>
  );
}
