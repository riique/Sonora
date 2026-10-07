import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X, Copy } from "lucide-react";

/**
 * Frameless window title bar — transparent overlay.
 *
 * It replaces the native Windows title bar (`decorations: false`) without
 * taking any vertical space: it is absolutely positioned over the top edge of
 * the existing layout, so the sidebar and the main content keep starting at the
 * very top of the window exactly as before. The bar itself is fully transparent
 * (no background, no border), which lets the app's own content show through
 * underneath — matching the seamless look of Discord / Spotify / VS Code.
 *
 * The whole top strip is a drag region (`data-tauri-drag-region`) so the window
 * can be moved by grabbing the top edge, and double-clicking it toggles the
 * maximized state (built-in Tauri behaviour). Only the three window controls on
 * the right capture clicks.
 */
export function TitleBar() {
  const [maximized, setMaximized] = useState(false);
  const appWindow = getCurrentWindow();

  // Keep the maximize/restore icon in sync with the real window state.
  // Listens to the OS-level `tauri://resize` event so the icon also updates
  // when the user maximizes via Aero Snap, keyboard shortcuts, etc.
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    (async () => {
      try {
        setMaximized(await appWindow.isMaximized());
        unlisten = await appWindow.onResized(async () => {
          setMaximized(await appWindow.isMaximized());
        });
      } catch {
        // Running outside of Tauri (e.g. plain `vite dev` in the browser).
      }
    })();

    return () => {
      unlisten?.();
    };
  }, [appWindow]);

  const handleMinimize = () => appWindow.minimize();
  const handleToggleMaximize = () => appWindow.toggleMaximize();
  const handleClose = () => appWindow.close();

  return (
    <div className="pointer-events-none absolute inset-x-0 top-0 z-50 flex h-9 items-stretch select-none">
      {/* Transparent drag region spanning the whole top edge. It overlays the
          sidebar brand and the main content header area, which are both
          non-interactive at the very top, so nothing clickable is hidden. */}
      <div
        data-tauri-drag-region
        className="pointer-events-auto flex-1"
      />

      {/* Window controls — these must capture clicks, so they are the only
          part of the overlay with real pointer events. */}
      <div className="pointer-events-auto flex items-center">
        <TitleBarButton
          label="Minimizar"
          onClick={handleMinimize}
          hoverClass="hover:bg-[#e8e8e3] hover:text-ink"
        >
          <Minus className="h-4 w-4" strokeWidth={2} />
        </TitleBarButton>

        <TitleBarButton
          label={maximized ? "Restaurar" : "Maximizar"}
          onClick={handleToggleMaximize}
          hoverClass="hover:bg-[#e8e8e3] hover:text-ink"
        >
          {maximized ? (
            <Copy className="h-3.5 w-3.5 -scale-x-100" strokeWidth={2} />
          ) : (
            <Square className="h-3 w-3" strokeWidth={2} />
          )}
        </TitleBarButton>

        <TitleBarButton
          label="Fechar"
          onClick={handleClose}
          hoverClass="hover:bg-[#e81123] hover:text-white"
        >
          <X className="h-4 w-4" strokeWidth={2} />
        </TitleBarButton>
      </div>
    </div>
  );
}

function TitleBarButton({
  label,
  onClick,
  hoverClass,
  children,
}: {
  label: string;
  onClick: () => void;
  hoverClass: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      // Prevent the drag region from intercepting the click.
      onMouseDown={(e) => e.stopPropagation()}
      className={
        "flex h-9 w-11 items-center justify-center text-muted transition-colors duration-150 " +
        hoverClass
      }
    >
      {children}
    </button>
  );
}
