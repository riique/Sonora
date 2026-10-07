import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { Button } from "../components/ui/Button";
import { PreferenceRow, RowGroup, Section } from "../components/ui/Surface";
import { Toggle } from "../components/ui/Toggle";
import { checkForUpdate, installUpdate, onUpdateReady, type UpdateInfo } from "../lib/tauri";
import { useFeatures } from "../lib/useFeatures";

/** Automatic updates from GitHub Releases. */
export function UpdatesSection() {
  const { features, update, error: featuresError } = useFeatures();
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [state, setState] = useState<"idle" | "checking" | "installing">("idle");
  const [message, setMessage] = useState("");
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion().then((value) => setVersion(value ?? "")).catch(() => {});
  }, []);

  useEffect(() => {
    const subscription = onUpdateReady(setInfo);
    return () => {
      subscription.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  const check = async () => {
    setState("checking");
    setMessage("");
    try {
      const next = await checkForUpdate();
      setInfo(next);
      setMessage(next.available ? "" : `Você está na versão mais recente (${next.current_version}).`);
    } catch (checkError) {
      setMessage(`Não foi possível verificar agora: ${String(checkError)}`);
    } finally {
      setState("idle");
    }
  };

  const install = async () => {
    setState("installing");
    setMessage("");
    try {
      await installUpdate();
    } catch (installError) {
      setMessage(String(installError));
      setState("idle");
    }
  };

  return (
    <Section title="Atualizações" description="Novas versões publicadas no GitHub são baixadas em segundo plano.">
      <RowGroup>
        <PreferenceRow
          title="Atualizar automaticamente"
          description="Instala ao abrir o Sonora, antes de você começar a ditar. Durante o uso, avisa e espera você reiniciar."
        >
          <Toggle
            label="Atualizar automaticamente"
            checked={features?.auto_update ?? true}
            disabled={!features}
            onChange={(value) => void update({ auto_update: value })}
          />
        </PreferenceRow>
        <PreferenceRow
          title={info?.available ? `Versão ${info.available} disponível` : "Versão instalada"}
          description={
            message || featuresError ||
            (info?.available
              ? "Reinicie para instalar. Leva poucos segundos."
              : (info?.current_version || version)
                ? `Sonora ${info?.current_version || version}.`
                : "Verifique se há uma versão nova.")
          }
        >
          {info?.available ? (
            <Button size="sm" variant="primary" disabled={state !== "idle"} onClick={() => void install()}>
              {state === "installing" ? "Reiniciando…" : "Reiniciar e atualizar"}
            </Button>
          ) : (
            <Button size="sm" disabled={state !== "idle"} onClick={() => void check()}>
              {state === "checking" ? "Verificando…" : "Verificar agora"}
            </Button>
          )}
        </PreferenceRow>
      </RowGroup>
    </Section>
  );
}
