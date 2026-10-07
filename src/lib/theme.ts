export type ThemePreference = "system" | "light" | "dark";

const STORAGE_KEY = "sonora.theme";
const media = () => (typeof window !== "undefined" && window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null);

export function getThemePreference(): ThemePreference {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "light" || stored === "dark" || stored === "system") return stored;
  } catch {
    // Storage can be unavailable; fall back to the system theme.
  }
  return "system";
}

function resolve(preference: ThemePreference): "light" | "dark" {
  if (preference !== "system") return preference;
  return media()?.matches ? "dark" : "light";
}

export function applyTheme(preference: ThemePreference = getThemePreference()) {
  document.documentElement.dataset.theme = resolve(preference);
}

export function setThemePreference(preference: ThemePreference) {
  try {
    localStorage.setItem(STORAGE_KEY, preference);
  } catch {
    // The choice still applies for this session.
  }
  applyTheme(preference);
}

/** Applies the stored theme now and follows Windows while the preference is "system". */
export function initTheme() {
  applyTheme();
  media()?.addEventListener("change", () => {
    if (getThemePreference() === "system") applyTheme("system");
  });
}
