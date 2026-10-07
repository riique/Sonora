import type { ReactNode } from "react";

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex min-w-7 items-center justify-center rounded-[7px] border border-[#d5d5cf] bg-canvas px-2 py-1 font-sans text-[11px] font-medium leading-4 text-[#444540] shadow-[0_1px_0_#c8c8c1]">
      {children}
    </kbd>
  );
}

export function KbdCombo({ keys }: { keys: string[] }) {
  return (
    <span className="inline-flex items-center gap-1.5" aria-label={keys.join(" mais ")}>
      {keys.map((k, i) => (
        <span key={`${k}-${i}`} className="inline-flex items-center gap-1.5">
          <Kbd>{k}</Kbd>
          {i < keys.length - 1 && (
            <span className="text-[11px] text-[#999a93]">+</span>
          )}
        </span>
      ))}
    </span>
  );
}
