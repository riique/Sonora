import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { insightsFixture } from "../fixtures/insights";
const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
import { VoiceInsights } from "../../src/views/VoiceInsights";
let host: HTMLDivElement;
let root: Root;
const reload = vi.fn(async () => {});
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  HTMLDialogElement.prototype.showModal = function () { this.open = true; };
  HTMLDialogElement.prototype.close = function () { this.open = false; };
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  bridge.invoke.mockReset(); reload.mockClear();
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

test("portrait is readable without exposing confidence, models or acoustic numbers upfront", async () => {
  await act(async () => root.render(<VoiceInsights data={insightsFixture()} reload={reload} developerMode={false} />));
  expect(host.querySelector("h2")?.textContent).toBe("Ideias que ganham forma");
  expect(host.querySelector("blockquote")?.textContent).toContain("Vamos por partes");
  expect(host.querySelectorAll(".voice-habits article")).toHaveLength(3);
  expect(host.querySelector<HTMLDetailsElement>(".voice-details")?.open).toBe(false);
  expect(host.querySelector(".voice-portrait")?.textContent).not.toMatch(/LUFS|confiança|OpenRouter|MATTR/);
  expect(bridge.invoke).not.toHaveBeenCalled();
});

test("AI generation waits for explicit confirmation and survives failure", async () => {
  const data = insightsFixture(); data.profile_generation_ready = true;
  bridge.invoke.mockRejectedValue(new Error("synthetic provider failure"));
  await act(async () => root.render(<VoiceInsights data={data} reload={reload} developerMode={false} />));
  await act(async () => host.querySelector<HTMLButtonElement>(".voice-portrait button")!.click());
  expect(host.querySelector<HTMLDialogElement>("dialog")?.open).toBe(true);
  expect(host.querySelector("dialog")?.textContent).toContain("meta/muse-spark-1.3-contributor");
  expect(bridge.invoke).not.toHaveBeenCalled();
  await act(async () => host.querySelectorAll<HTMLButtonElement>("dialog button")[1].click());
  expect(bridge.invoke).toHaveBeenCalledWith("generate_ai_voice_profile");
  expect(host.textContent).toContain("Não foi possível criar seu retrato");
  expect(host.querySelector("h2")?.textContent).toBe("Ideias que ganham forma");
  expect(host.querySelector<HTMLButtonElement>(".voice-portrait button")?.disabled).toBe(false);
});

test("update stays visible with words remaining and enables only at the backend milestone", async () => {
  const data = insightsFixture();
  await act(async () => root.render(<VoiceInsights data={data} reload={reload} developerMode={false} />));
  const button = host.querySelector<HTMLButtonElement>(".voice-portrait button")!;
  expect(button.textContent).toBe("Atualizar meu retrato");
  expect(button.disabled).toBe(true);
  expect(host.querySelector(`#${button.getAttribute("aria-describedby")}`)?.textContent).toBe("Faltam 760 palavras ditadas para atualizar seu retrato.");
  await act(async () => button.click());
  expect(host.querySelector("dialog")).toBeNull();
  expect(bridge.invoke).not.toHaveBeenCalled();

  data.profile_progress_words = 1999;
  await act(async () => root.render(<VoiceInsights data={{...data}} reload={reload} developerMode={false} />));
  expect(host.textContent).toContain("Falta 1 palavra ditada para atualizar seu retrato.");
  expect(button.disabled).toBe(true);

  data.profile_progress_words = 2000; data.profile_generation_ready = true;
  await act(async () => root.render(<VoiceInsights data={{...data}} reload={reload} developerMode={false} />));
  expect(button.disabled).toBe(false);
  expect(host.textContent).toContain("Você já pode atualizar seu retrato.");
  expect(bridge.invoke).not.toHaveBeenCalled();
});

test("first portrait shows its own remaining words without claiming automatic generation", async () => {
  const data = insightsFixture(false); data.profile_progress_words = 120; data.profile_required_words = 500;
  await act(async () => root.render(<VoiceInsights data={data} reload={reload} developerMode={false} />));
  const button = host.querySelector<HTMLButtonElement>(".voice-portrait button")!;
  expect(button.textContent).toBe("Criar meu retrato");
  expect(button.disabled).toBe(true);
  expect(host.textContent).toContain("Faltam 380 palavras ditadas para criar seu primeiro retrato.");
  expect(bridge.invoke).not.toHaveBeenCalled();
});

test("empty profile and disabled AI retain local discoveries and optional preferences", async () => {
  const data = insightsFixture(false); data.profile_enabled = false;
  await act(async () => root.render(<VoiceInsights data={data} reload={reload} developerMode={false} />));
  expect(host.textContent).toContain("Seu jeito de falar vai aparecer aqui");
  expect(host.textContent).toContain("Vamos por partes");
  expect(host.querySelector(".voice-portrait button")).toBeNull();
  expect(host.querySelector('[role="switch"]')?.getAttribute("aria-checked")).toBe("false");
  expect(bridge.invoke).not.toHaveBeenCalled();
});

test("vocabulary remains actionable under details and preserves error feedback", async () => {
  bridge.invoke.mockResolvedValue(undefined);
  await act(async () => root.render(<VoiceInsights data={insightsFixture()} reload={reload} developerMode={false} />));
  const add = [...host.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Adicionar ao vocabulário")!;
  await act(async () => add.click());
  expect(bridge.invoke).toHaveBeenCalledWith("add_insight_correction_to_vocabulary", { before: "planejar", after: "planejamento" });
  expect(reload).toHaveBeenCalledOnce();
});
