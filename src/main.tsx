import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { GadgetApp, GadgetPreviewApp } from "./views/GadgetView";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { initTheme } from "./lib/theme";
import "./index.css";

// The same bundle is loaded by both the main window and the always-on-top
// gadget overlay; we tell them apart by the Tauri window label and render a
// different root for each. Falls back to the main app outside of Tauri.
let windowLabel = "main";
try {
  windowLabel = getCurrentWindow().label;
} catch {
  // Not running inside Tauri (e.g. plain `vite` dev) — assume the main app.
}

const isGadget = windowLabel === "gadget";
const isGadgetPreview = import.meta.env.DEV && new URLSearchParams(window.location.search).has("gadgetPreview");

if (isGadget) {
  // The gadget window must be see-through so only its pill is painted.
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
} else {
  initTheme();
}

async function renderApp() {
  let content = isGadget ? <GadgetApp /> : isGadgetPreview ? <GadgetPreviewApp /> : <App />;
  if (import.meta.env.DEV && new URLSearchParams(window.location.search).has("insightsPreview")) {
    const { InsightsPreview } = await import("./dev/InsightsPreview");
    content = <InsightsPreview />;
  }
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      {content}
    </ErrorBoundary>
  </React.StrictMode>,
);
}
void renderApp();

if (import.meta.env.PROD) {
  document.addEventListener("contextmenu", (e) => e.preventDefault());
}
