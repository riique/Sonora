import { useCallback, useEffect, useState } from "react";
import { getFeatureSettings, setFeatureSettings, type FeatureSettings } from "./tauri";

/** Loads the assist feature toggles and saves each change immediately. */
export function useFeatures() {
  const [features, setFeatures] = useState<FeatureSettings | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    getFeatureSettings().then(setFeatures).catch((loadError) => setError(String(loadError)));
  }, []);

  const update = useCallback(
    async (patch: Partial<FeatureSettings>) => {
      if (!features) return;
      const previous = features;
      const next = { ...features, ...patch };
      setFeatures(next);
      setError("");
      try {
        setFeatures(await setFeatureSettings(next));
      } catch (saveError) {
        setFeatures(previous);
        setError(String(saveError));
      }
    },
    [features],
  );

  return { features, update, error };
}
