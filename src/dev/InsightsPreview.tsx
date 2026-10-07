import { useState } from "react";
import { VoiceInsights } from "../views/VoiceInsights";
import { insightsFixture } from "../../tests/fixtures/insights";

/** Only dynamically imported in development. No user data or provider calls. */
export function InsightsPreview() {
  const [empty, setEmpty] = useState(false);
  const data = insightsFixture(!empty);
  if (empty) { data.voice_evidence.signature_candidates = {}; data.voice_evidence.recurring_topics = []; }
  return <main className="mx-auto max-w-[1100px] px-8 py-12">
    <div className="mb-8 flex flex-wrap items-center justify-between gap-4 border-b border-line pb-5"><span className="text-[17px] font-semibold">Sonora · Prévia com dados fictícios</span><button className="text-[13px] underline" onClick={() => setEmpty(!empty)}>{empty ? "Ver retrato" : "Ver estado inicial"}</button></div>
    <h1 className="text-[28px] font-semibold tracking-tight">Sua voz</h1>
    <p className="mt-2 text-[13px] text-muted">Seu jeito de falar, em poucas palavras.</p>
    <VoiceInsights data={data} reload={async () => {}} developerMode={false} />
  </main>;
}
