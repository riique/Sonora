import { useEffect, useState } from "react";
import { Button } from "../components/ui/Button";
import { PreferenceRow, RowGroup, Section, Segmented } from "../components/ui/Surface";
import { Toggle } from "../components/ui/Toggle";
import {
  downloadLocalModel,
  getLocalModelStatus,
  onLocalModelProgress,
  removeLocalModel,
  type FeatureSettings,
  type LocalModel,
  type LocalModelProgress,
  type LocalModelStatus,
} from "../lib/tauri";

const MODELS: Record<LocalModel, { label: string; size: string; note: string }> = {
  base: { label: "Leve", size: "142 MB", note: "Mais rápido; erra mais em nomes e termos técnicos." },
  small: { label: "Equilibrado", size: "466 MB", note: "Melhor para português; leva alguns segundos por ditado." },
};

const megabytes = (bytes: number) => `${Math.round(bytes / 1_000_000).toLocaleString("pt-BR")} MB`;

/** Offline fallback: whisper.cpp on this computer when the cloud fails. */
export function OfflineSection({
  features,
  onChange,
}: {
  features: FeatureSettings | null;
  onChange: (patch: Partial<FeatureSettings>) => Promise<void> | void;
}) {
  const model = features?.offline_model ?? "small";
  const [status, setStatus] = useState<LocalModelStatus | null>(null);
  const [progress, setProgress] = useState<LocalModelProgress | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    setStatus(null);
    getLocalModelStatus(model).then(setStatus).catch((loadError) => setError(String(loadError)));
  }, [model]);

  useEffect(() => {
    const subscription = onLocalModelProgress((next) => {
      if (next.model === model) setProgress(next);
    });
    return () => {
      subscription.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [model]);

  const downloading = Boolean(status?.downloading || progress);
  const percent = progress?.total ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100)) : null;

  const download = async () => {
    setError("");
    setProgress({ model, downloaded: 0, total: null });
    try {
      setStatus(await downloadLocalModel(model));
    } catch (downloadError) {
      setError(String(downloadError));
      setStatus(await getLocalModelStatus(model).catch(() => null));
    } finally {
      setProgress(null);
    }
  };

  const remove = async () => {
    setError("");
    try {
      setStatus(await removeLocalModel(model));
    } catch (removeError) {
      setError(String(removeError));
    }
  };

  return (
    <Section
      title="Sem internet"
      description="Se nenhum provedor responder, o ditado é transcrito neste computador. O áudio não sai da máquina."
    >
      <RowGroup>
        <PreferenceRow title="Transcrever offline quando a nuvem falhar" description="Funciona só com o modelo baixado abaixo.">
          <Toggle
            label="Transcrever offline quando a nuvem falhar"
            checked={features?.offline_fallback ?? true}
            disabled={!features}
            onChange={(value) => void onChange({ offline_fallback: value })}
          />
        </PreferenceRow>
        <PreferenceRow title="Modelo local" description={MODELS[model].note}>
          <Segmented<LocalModel>
            label="Modelo local"
            value={model}
            onChange={(next) => void onChange({ offline_model: next })}
            options={(Object.keys(MODELS) as LocalModel[]).map((value) => ({ value, label: `${MODELS[value].label} · ${MODELS[value].size}` }))}
          />
        </PreferenceRow>
        <div className="py-4">
          <div className="flex items-center justify-between gap-10">
            <div className="min-w-0">
              <h3 className="text-[13px] font-medium text-ink">
                {status?.installed ? "Modelo instalado" : downloading ? "Baixando modelo" : "Modelo não baixado"}
              </h3>
              <p className="mt-0.5 text-[12.5px] leading-5 text-muted tabular-nums" aria-live="polite">
                {status?.installed && status.size_bytes
                  ? `${megabytes(status.size_bytes)} neste computador.`
                  : downloading
                    ? progress?.total
                      ? `${megabytes(progress.downloaded)} de ${megabytes(progress.total)}`
                      : "Conectando…"
                    : `Download único de ${MODELS[model].size}.`}
              </p>
            </div>
            {status?.installed ? (
              <Button size="sm" variant="danger" onClick={() => void remove()}>
                Remover
              </Button>
            ) : (
              <Button size="sm" disabled={downloading || !status} onClick={() => void download()}>
                {downloading ? "Baixando…" : "Baixar modelo"}
              </Button>
            )}
          </div>
          {downloading && (
            <div
              className="download-meter mt-3"
              role="progressbar"
              aria-label="Download do modelo offline"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={percent ?? undefined}
            >
              <span style={{ width: `${percent ?? 4}%` }} />
            </div>
          )}
          {error && <p className="mt-2 text-[12px] text-live" role="alert">{error}</p>}
        </div>
      </RowGroup>
    </Section>
  );
}
