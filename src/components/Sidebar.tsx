import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { ChartNoAxesColumn, History, House, Settings2, type LucideIcon } from "lucide-react";
import type { ViewKey } from "../views";
import { ON_AIR_LABEL, Tally, type OnAirState } from "../recording/useOnAir";

const NAV: { key: ViewKey; label: string; icon: LucideIcon }[] = [
  { key: "inicio", label: "Início", icon: House },
  { key: "historico", label: "Histórico", icon: History },
  { key: "insights", label: "Insights", icon: ChartNoAxesColumn },
  { key: "ajustes", label: "Ajustes", icon: Settings2 },
];

export function Sidebar({
  current,
  onSelect,
  onAir,
}: {
  current: ViewKey;
  onSelect: (view: ViewKey) => void;
  onAir: OnAirState;
}) {
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(""));
  }, []);

  return (
    <aside className="flex w-[220px] shrink-0 flex-col border-r border-line bg-sidebar pb-5 pt-12 max-[900px]:w-[64px]">
      <div className="flex h-9 items-center justify-between px-6 max-[900px]:justify-center max-[900px]:px-0">
        <span className="font-display text-[17px] font-semibold tracking-[-0.025em] text-ink max-[900px]:hidden">Sonora</span>
        <span className="flex items-center gap-2" role="status" aria-label={`Estado: ${ON_AIR_LABEL[onAir]}`} title={ON_AIR_LABEL[onAir]}>
          <span className={"text-[11px] font-medium max-[900px]:hidden " + (onAir === "live" ? "text-live" : onAir === "standby" ? "text-standby" : "text-transparent")} aria-hidden>
            {onAir === "off" ? "" : ON_AIR_LABEL[onAir]}
          </span>
          <Tally state={onAir} />
        </span>
      </div>

      <nav className="mt-9 flex flex-1 flex-col gap-0.5 px-3" aria-label="Navegação principal">
        {NAV.map((item) => {
          const Icon = item.icon;
          const active = current === item.key;
          return (
            <button
              key={item.key}
              type="button"
              aria-current={active ? "page" : undefined}
              title={item.label}
              onClick={() => onSelect(item.key)}
              className={
                "flex h-9 items-center gap-3 rounded-[8px] px-3 text-[13px] transition-colors duration-150 max-[900px]:justify-center max-[900px]:px-0 " +
                (active ? "bg-fill font-medium text-ink" : "text-muted hover:bg-fill/60 hover:text-ink")
              }
            >
              <Icon className={"h-4 w-4 shrink-0 " + (active ? "text-ink" : "")} strokeWidth={1.75} aria-hidden />
              <span className="max-[900px]:sr-only">{item.label}</span>
            </button>
          );
        })}
      </nav>

      {version && (
        <div className="timecode px-6 max-[900px]:px-0 max-[900px]:text-center">v{version}</div>
      )}
    </aside>
  );
}
