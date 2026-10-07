import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
const bridge = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn(), drop: undefined as undefined | ((event: unknown) => void), listeners: new Map<string, (event: unknown) => void>() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name: string, callback: (event: unknown) => void) => { bridge.listeners.set(name, callback); return () => bridge.listeners.delete(name); }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: bridge.open, save: vi.fn() }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async (callback: (event: unknown) => void) => { bridge.drop = callback; return () => { bridge.drop = undefined; }; } }) }));
import { ScratchpadView } from "../../src/views/ScratchpadView";
import { IntelligenceSettings } from "../../src/views/IntelligenceSettings";
import { TranscricaoView } from "../../src/views/TranscricaoView";
import { InicioView } from "../../src/views/InicioView";
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  bridge.invoke.mockReset(); bridge.open.mockReset(); bridge.listeners.clear();
});
test("upload keeps the admitted file stable while processing and exposes cancellation", async () => {
  let complete!: (text: string) => void;
  bridge.open.mockResolvedValue("C:/synthetic/first.wav");
  bridge.invoke.mockImplementation((command: string) => command === "transcribe_file" ? new Promise<string>((resolve) => { complete = resolve; }) : Promise.resolve());
  await act(async () => root.render(<TranscricaoView />));
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-describedby="upload-formats"]')!.click());
  await act(async () => [...host.querySelectorAll("button")].find((button) => button.textContent === "Transcrever arquivo")!.click());
  await act(async () => bridge.drop?.({ payload: { type: "drop", paths: ["C:/synthetic/second.wav"] } }));
  expect(host.textContent).toContain("first.wav"); expect(host.textContent).not.toContain("second.wav");
  expect(host.textContent).toContain("Cancelar transcrição");
  expect(bridge.invoke.mock.calls.filter(([name]) => name === "transcribe_file")).toHaveLength(1);
  await act(async () => complete("Texto sintético"));
  expect(host.textContent).toContain("Texto sintético");
});
test("file dialog failure becomes visible", async () => {
  bridge.open.mockRejectedValue(new Error("synthetic dialog failure"));
  await act(async () => root.render(<TranscricaoView />));
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-describedby="upload-formats"]')!.click());
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("synthetic dialog failure");
});
test("home renders the persisted clipboard destination with labeled quick controls", async () => {
  bridge.invoke.mockImplementation(async (command: string) => ({
    get_history_page: { items: [], total: 0, total_words: 0 },
    get_mode_config: { mode: "ultra-fast", gemini_pipelines: { ultra_fast_whisper: "large-v3" } },
    get_output_policy_config: { formatting_level: "smart", destination: "clipboard_only", profiles: [], temporary_override: null },
    get_shortcuts: { toggle: "Control+B", cancel: "Control+Q" },
    get_recording_status: { phase: "idle", recording: false, revision: 0 },
  }[command]));
  await act(async () => root.render(<InicioView onNavigate={() => {}} />));
  const controls = host.querySelectorAll("select");
  expect(controls[0].value).toBe("clipboard_only");
  for (const control of controls) expect(control.closest("label")?.textContent).toBeTruthy();
  expect(host.textContent).toContain("Ultrarrápido");
  expect(host.textContent).toContain("Pronto para ditar");
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
test("initial preference failure exposes an actionable error instead of permanent loading", async () => {
  bridge.invoke.mockRejectedValue(new Error("synthetic storage failure"));
  await act(async () => root.render(<IntelligenceSettings />));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Não foi possível carregar");
  expect(host.textContent).toContain("Recarregar preferências");
});
test("scratchpad refreshes after a saved dictation and reports clipboard failure", async () => {
  let notes: unknown[] = [];
  bridge.invoke.mockImplementation(async (name: string) => { if (name === "get_scratchpad_notes") return notes; throw new Error(`Unexpected command: ${name}`); });
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn().mockRejectedValue(new Error("clipboard busy")) } });
  await act(async () => root.render(<ScratchpadView />));
  expect(host.textContent).toContain("Nenhuma nota rápida");
  notes = [{ id: "synthetic-note", text: "Nota de teste", created_at_ms: 1 }];
  await act(async () => { bridge.listeners.get("transcription-saved")?.({}); });
  expect(host.textContent).toContain("Nota de teste");
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Copiar nota"]')!.click());
  expect(host.textContent).toContain("Não foi possível copiar");
});
test("scratchpad load failure is visible and preserves a retry action", async () => {
  bridge.invoke.mockRejectedValue(new Error("synthetic read failure"));
  await act(async () => root.render(<ScratchpadView />));
  expect(host.textContent).toContain("Não foi possível carregar as notas");
  expect(host.textContent).toContain("Atualizar notas");
});
