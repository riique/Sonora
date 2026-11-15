import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
const bridge = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn(), drop: undefined as undefined | ((event: unknown) => void), listeners: new Map<string, (event: unknown) => void>() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name: string, callback: (event: unknown) => void) => { bridge.listeners.set(name, callback); return () => bridge.listeners.delete(name); }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: bridge.open, save: vi.fn() }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async (callback: (event: unknown) => void) => { bridge.drop = callback; return () => { bridge.drop = undefined; }; } }) }));
import { InterruptedAudio } from "../../src/views/RecoveryView";
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
test("home shows the single transcription route and quick shortcuts", async () => {
  bridge.invoke.mockImplementation(async (command: string) => ({
    get_history_page: { items: [], total: 0, total_words: 0 },
    get_transcription_route: { provider: "open-router", model: "transcribe35" },
    get_shortcuts: { toggle: "Control+B", cancel: "Control+Q" },
    get_recording_status: { phase: "idle", recording: false, revision: 0 },
  }[command]));
  await act(async () => root.render(<InicioView onNavigate={() => {}} />));
  expect(host.querySelectorAll("select")).toHaveLength(0);
  expect(host.textContent).toContain("Gemini 3.5 Transcribe via OpenRouter");
  expect(host.textContent).toContain("Pronto para ditar");
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
test("interrupted recordings stay hidden when there is nothing to recover", async () => {
  bridge.invoke.mockImplementation(async (name: string) => { if (name === "list_recovery_audio") return []; throw new Error(`Unexpected command: ${name}`); });
  await act(async () => root.render(<InterruptedAudio />));
  expect(host.textContent).toBe("");
});
test("interrupted recordings can be retranscribed from the history", async () => {
  let pending = [{ id: "recovery-synthetic", bytes: 2 * 1048576 }];
  bridge.invoke.mockImplementation(async (name: string) => {
    if (name === "list_recovery_audio") return pending;
    if (name === "retry_recovery_audio") { pending = []; return "texto"; }
    throw new Error(`Unexpected command: ${name}`);
  });
  const recovered = vi.fn();
  await act(async () => root.render(<InterruptedAudio onRecovered={recovered} />));
  expect(host.textContent).toContain("1 gravação interrompida");
  await act(async () => [...host.querySelectorAll("button")].find((button) => button.textContent === "Retranscrever")!.click());
  expect(recovered).toHaveBeenCalled();
  expect(bridge.invoke.mock.calls.some(([name]) => name === "retry_recovery_audio")).toBe(true);
});
