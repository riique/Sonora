import type { ReactNode } from "react";
import { ArrowRight } from "lucide-react";

/**
 * The studio log row. Every list of takes uses the same anatomy, in the same
 * places: time over duration · text · actions.
 */
export const LOG_GRID = "grid grid-cols-[52px_minmax(0,1fr)_104px] gap-x-5";

export function LogStamp({ time, duration }: { time: string; duration?: string }) {
  return (
    <div className="timecode pt-px">
      <div className="text-soft">{time}</div>
      {duration && <div>{duration}</div>}
    </div>
  );
}

export function LogRow({
  time,
  duration,
  children,
  onOpen,
}: {
  time: string;
  duration?: string;
  children: ReactNode;
  onOpen: () => void;
}) {
  return (
    <li>
      <button type="button" onClick={onOpen} className={"group w-full py-3.5 text-left transition-colors hover:bg-fill/40 focus-visible:bg-fill/40 " + LOG_GRID}>
        <LogStamp time={time} duration={duration} />
        <div className="min-w-0 pt-px">{children}</div>
        <span className="reveal-on-row flex justify-end pt-0.5 text-muted" aria-hidden>
          <ArrowRight className="h-4 w-4" />
        </span>
      </button>
    </li>
  );
}
