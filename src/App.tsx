import { useRef, useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { InicioView } from "./views/InicioView";
import { HistoricoView } from "./views/HistoricoView";
import { ConfiguracoesView } from "./views/ConfiguracoesView";
import { InsightsView } from "./views/InsightsView";
import { useOnAir } from "./recording/useOnAir";
import { Notices } from "./components/Notices";
import type { Navigate, SettingsTab, ViewKey } from "./views";

export default function App() {
  const [view, setView] = useState<ViewKey>("inicio");
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("geral");
  const main = useRef<HTMLElement | null>(null);
  const onAir = useOnAir();

  const navigate: Navigate = (next, tab) => {
    if (tab) setSettingsTab(tab);
    setView(next);
    main.current?.scrollTo({ top: 0 });
  };

  return (
    <div className="relative flex h-screen w-screen overflow-hidden bg-canvas text-ink">
      <a href="#main-content" className="skip-link">Pular para o conteúdo</a>
      <TitleBar />

      <Sidebar current={view} onSelect={(next) => navigate(next)} onAir={onAir} />

      <main ref={main} id="main-content" tabIndex={-1} className="scrollbar-thin min-w-0 flex-1 overflow-y-auto outline-none">
        <div key={view} className="page-shell animate-fade-in">
          <div className={view === "ajustes" ? "" : "max-w-[784px]"}>
          {view === "inicio" && <InicioView onNavigate={navigate} onAir={onAir} />}
          {view === "historico" && <HistoricoView onNavigate={navigate} />}
          {view === "insights" && <InsightsView />}
          {view === "ajustes" && <ConfiguracoesView tab={settingsTab} onTabChange={setSettingsTab} />}
          </div>
        </div>
      </main>
      <Notices />
    </div>
  );
}
