import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

import { ConfiguracoesView } from "../../src/views/ConfiguracoesView";
import { routeSummary } from "../../src/views/InicioView";

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  bridge.invoke.mockReset();
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

test("Transcrição settings remove Google AI Studio, Gemini and Smart Context, expose AssemblyAI-STT-3.7 direct and OpenRouter custom IDs", async () => {
  let savedRoute = {
    provider: "assemblyai",
    model: "assemblyai-stt-3-7",
    use_custom_model: false,
    custom_model: "",
  };

  bridge.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    if (command === "get_transcription_route") return savedRoute;
    if (command === "set_transcription_route") {
      savedRoute = args?.route as typeof savedRoute;
      return savedRoute;
    }
    if (command === "get_api_keys") return { assemblyai: ["stored:1234"], openrouter: [] };
    return null;
  });

  await act(async () => root.render(<ConfiguracoesView tab="transcricao" />));

  // Google AI Studio, Gemini and Contexto Inteligente must not appear anywhere.
  expect(host.textContent).not.toContain("Google AI Studio");
  expect(host.textContent).not.toContain("Gemini");
  expect(host.textContent).not.toContain("Contexto Inteligente");

  // AssemblyAI direct with AssemblyAI-STT-3.7 and its own API key must be present.
  expect(host.textContent).toContain("AssemblyAI (API direta)");
  expect(host.textContent).toContain("AssemblyAI-STT-3.7 (assemblyai-stt-3-7)");
  expect(host.textContent).toContain("1 chave");

  // Switch provider to OpenRouter: Universal-3.6 Pro and custom model ID option appear.
  const providerSelect = host.querySelector<HTMLSelectElement>("#route-provider")!;
  expect([...providerSelect.options].map((o) => o.textContent)).toContain("ElevenLabs (API direta)");
  expect(host.textContent).toContain("Transcrição direta com ElevenLabs Scribe v3");
  await act(async () => {
    providerSelect.value = "open-router";
    providerSelect.dispatchEvent(new Event("change", { bubbles: true }));
  });

  const modelSelect = host.querySelector<HTMLSelectElement>("#route-model")!;
  const options = [...modelSelect.options].map((o) => o.textContent);
  expect(options).toContain("AssemblyAI Universal-3.6 Pro");
  expect(options).toContain("ElevenLabs Scribe v2");
  expect(options).toContain("ID customizado do OpenRouter…");
  expect(options.some((label) => label?.toLowerCase().includes("gemini"))).toBe(false);
  expect(options.some((label) => label?.toLowerCase().includes("openai"))).toBe(false);

  // Select custom OpenRouter model ID and save it.
  await act(async () => {
    modelSelect.value = "__custom__";
    modelSelect.dispatchEvent(new Event("change", { bubbles: true }));
  });

  const customInput = host.querySelector<HTMLInputElement>("#route-custom-model")!;
  expect(customInput).not.toBeNull();
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    setter.call(customInput, "mistralai/voxtral-small-24b-2507");
    customInput.dispatchEvent(new Event("input", { bubbles: true }));
  });

  const saveButton = [...host.querySelectorAll("button")].find((b) => b.textContent === "Salvar")!;
  await act(async () => saveButton.click());

  const setCalls = bridge.invoke.mock.calls.filter(([cmd]) => cmd === "set_transcription_route");
  const lastCall = setCalls[setCalls.length - 1][1] as { route: typeof savedRoute };
  expect(lastCall.route.provider).toBe("open-router");
  expect(lastCall.route.use_custom_model).toBe(true);
  expect(lastCall.route.custom_model).toBe("mistralai/voxtral-small-24b-2507");
});

test("routeSummary formats direct AssemblyAI-STT-3.7, Universal-3.6 Pro, and custom OpenRouter IDs", () => {
  expect(
    routeSummary({ provider: "assemblyai", model: "assemblyai-stt-3-7", use_custom_model: false, custom_model: "" }),
  ).toBe("AssemblyAI-STT-3.7 via AssemblyAI");
  expect(
    routeSummary({ provider: "open-router", model: "assemblyai-universal-3-6-pro", use_custom_model: false, custom_model: "" }),
  ).toBe("AssemblyAI Universal-3.6 Pro via OpenRouter");
  expect(
    routeSummary({ provider: "open-router", model: "assemblyai-universal-3-6-pro", use_custom_model: true, custom_model: "openai/whisper-large-v3" }),
  ).toBe("openai/whisper-large-v3 via OpenRouter");
});
