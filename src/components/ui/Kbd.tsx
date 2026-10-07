import type { ReactNode } from "react";

export function Kbd({ children, size = "sm" }: { children: ReactNode; size?: "sm" | "lg" }) {
  return (
    <kbd
      className={
        "inline-flex items-center justify-center border border-line-strong bg-raised font-mono font-medium text-ink shadow-[0_1.5px_0_var(--n5)] " +
        (size === "lg"
          ? "h-11 min-w-11 rounded-[10px] px-3.5 text-[17px]"
          : "h-6 min-w-6 rounded-[6px] px-1.5 text-[11px]")
      }
    >
      {children}
    </kbd>
  );
}

export function KbdCombo({ keys, size = "sm" }: { keys: string[]; size?: "sm" | "lg" }) {
  return (
    <span className={"inline-flex items-center " + (size === "lg" ? "gap-2.5" : "gap-1")} aria-label={keys.join(" mais ")}>
      {keys.map((k, i) => (
        <span key={`${k}-${i}`} className={"inline-flex items-center " + (size === "lg" ? "gap-2.5" : "gap-1")}>
          <Kbd size={size}>{k}</Kbd>
          {i < keys.length - 1 && <span className={"text-faint " + (size === "lg" ? "text-[15px]" : "text-[11px]")} aria-hidden>+</span>}
        </span>
      ))}
    </span>
  );
}

export function shortcutKeys(shortcut: string): string[] {
  const labels: Record<string, string> = { Control: "Ctrl", CommandOrControl: "Ctrl", Alt: "Alt", Shift: "Shift", Super: "Win", Meta: "Win" };
  return shortcut.split("+").map((key) => labels[key] ?? key);
}
