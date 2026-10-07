import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { MoreHorizontal } from "lucide-react";

export type MenuAction = {
  label: string;
  onSelect: () => void;
  icon?: ReactNode;
  danger?: boolean;
  disabled?: boolean;
  hidden?: boolean;
};

/**
 * Overflow menu: one quiet "…" trigger instead of a row of icon buttons.
 * Arrow keys move between items, Escape returns focus to the trigger.
 */
export function Menu({ label, actions, trigger, align = "right" }: { label: string; actions: MenuAction[]; trigger?: ReactNode; align?: "left" | "right" }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement | null>(null);
  const button = useRef<HTMLButtonElement | null>(null);
  const id = useId();
  const visible = actions.filter((action) => !action.hidden);

  useEffect(() => {
    if (!open) return;
    root.current?.querySelector<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')?.focus();
    const close = (event: PointerEvent) => {
      if (root.current && !root.current.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open]);

  if (!visible.length) return null;

  return (
    <div className="relative" ref={root}>
      <button
        ref={button}
        type="button"
        className={trigger ? "inline-flex" : "icon-button"}
        onClick={() => setOpen((current) => !current)}
        aria-label={label}
        title={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
      >
        {trigger ?? <MoreHorizontal className="h-4 w-4" aria-hidden />}
      </button>
      {open && (
        <div
          id={id}
          className={"menu-panel animate-fade-in " + (align === "left" ? "left-0 right-auto" : "")}
          role="menu"
          onKeyDown={(event) => {
            const items = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)"));
            const index = items.indexOf(document.activeElement as HTMLButtonElement);
            if (event.key === "Escape") {
              event.preventDefault();
              setOpen(false);
              button.current?.focus();
            }
            if (event.key === "Tab") setOpen(false);
            if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
              event.preventDefault();
              const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : (index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
              items[next]?.focus();
            }
          }}
        >
          {visible.map((action, index) => (
            <div key={action.label}>
              {action.danger && index > 0 && <div className="mx-2 my-1 h-px bg-line" role="separator" />}
              <button
                type="button"
                role="menuitem"
                disabled={action.disabled}
                className={"menu-item " + (action.danger ? "text-live hover:bg-live-wash focus-visible:bg-live-wash" : "")}
                onClick={() => {
                  setOpen(false);
                  action.onSelect();
                }}
              >
                {action.icon && <span className="flex h-4 w-4 items-center justify-center [&>svg]:h-4 [&>svg]:w-4" aria-hidden>{action.icon}</span>}
                {action.label}
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
