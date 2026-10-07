import { useEffect, useRef, useState } from "react";
import { getRecordingElapsed, getRecordingStatus, onRecordingEvent, type RecordingStatus } from "../lib/tauri";
import { shouldApplyRecordingStatus } from "./status";

export type OnAirState = "off" | "live" | "standby";

export function onAirState(status: Pick<RecordingStatus, "phase" | "recording" | "busy"> | null): OnAirState {
  if (!status) return "off";
  if (status.recording || status.phase === "starting" || status.phase === "recording") return "live";
  if (status.busy || status.phase === "stopping" || status.phase === "cancelling") return "standby";
  return "off";
}

/** Follows the backend recording lifecycle; drives the tally lamp. */
export function useOnAir(): OnAirState {
  const [status, setStatus] = useState<RecordingStatus | null>(null);
  const latestRevision = useRef(-1);

  useEffect(() => {
    let mounted = true;
    let unlisten: (() => void) | undefined;
    const apply = (next: RecordingStatus) => {
      if (!mounted || !shouldApplyRecordingStatus(latestRevision.current, next)) return;
      latestRevision.current = next.revision;
      setStatus(next);
    };
    void onRecordingEvent((_type, next) => apply(next))
      .then(async (dispose) => {
        if (!mounted) {
          dispose();
          return;
        }
        unlisten = dispose;
        apply(await getRecordingStatus());
      })
      .catch((error) => console.error("Failed to sync recording status:", error));
    return () => {
      mounted = false;
      unlisten?.();
    };
  }, []);

  return onAirState(status);
}

/** Broadcast-clock reading of the live take, polled from the backend while on air. */
export function useElapsed(state: OnAirState): number {
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    if (state !== "live") {
      setElapsed(0);
      return;
    }
    let active = true;
    const tick = () => getRecordingElapsed().then((ms) => { if (active) setElapsed(ms); }).catch(() => {});
    void tick();
    const timer = window.setInterval(tick, 250);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [state]);
  return elapsed;
}

export function formatClock(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  return `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}

export const ON_AIR_LABEL: Record<OnAirState, string> = {
  off: "Pronto",
  live: "Gravando",
  standby: "Processando",
};

export function Tally({ state, size = "sm" }: { state: OnAirState; size?: "sm" | "lg" }) {
  const className = "tally" + (state === "live" ? " tally--live" : state === "standby" ? " tally--standby" : "") + (size === "lg" ? " tally--lg" : "");
  return <span className={className} aria-hidden />;
}
