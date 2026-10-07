pub mod assist;
pub mod audio;
pub mod audio_processing;
pub mod audio_store;
pub mod capture_spool;
pub mod commands;
pub mod context;
pub mod gemini;
pub mod groq;
pub mod history;
pub mod insights;
pub mod insights_intelligence;
pub mod learning;
pub mod local_whisper;
pub mod maintenance;
pub mod meta_asr;
pub mod mic_control;
pub mod models;
pub mod native_messaging;
pub mod openrouter;
pub mod operations;
pub mod output_policy;
pub mod pipeline_contract;
pub mod pipeline_run;
pub mod redaction;
pub mod sanitizer_json;
pub mod scratchpad;
pub mod secrets;
pub mod settings;
pub mod shortcuts;
pub mod single_instance;
pub mod snippets;
pub mod speech_presence;
pub mod storage;
pub mod transcription;
pub mod transformations;
pub mod updates;
pub mod vocabulary;

use models::{
    GadgetPresentation, GadgetSessionAnchor, GadgetVisualState, SharedState, WidgetVisibilityMode,
};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicIsize, Ordering};
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

/// Cached gadget HWND (raw pointer bits). `0` means unknown / invalid.
/// Refreshed when the gadget is created and whenever we successfully apply a
/// click-through toggle on the main thread. Used so the cursor watcher can do
/// pure Win32 hit-testing **off** the Tauri event loop.
#[cfg(target_os = "windows")]
static GADGET_HWND: AtomicIsize = AtomicIsize::new(0);

#[cfg(target_os = "windows")]
fn cache_gadget_hwnd(window: &tauri::WebviewWindow) {
    if let Ok(hwnd) = window.hwnd() {
        GADGET_HWND.store(hwnd.0 as isize, Ordering::Release);
    }
}

#[cfg(not(target_os = "windows"))]
fn cache_gadget_hwnd(_window: &tauri::WebviewWindow) {}

const GADGET_BOTTOM_MARGIN: f64 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct GadgetGeometry {
    width: f64,
    height: f64,
}

fn gadget_hit_rect(state: GadgetVisualState) -> Option<crate::models::GadgetHitRect> {
    use GadgetVisualState::*;

    let (window, pill) = match state {
        Hidden => return None,
        Idle => ((72.0, 52.0), (52.0, 32.0)),
        Hover => ((142.0, 58.0), (126.0, 38.0)),
        Appearing => ((72.0, 52.0), (54.0, 36.0)),
        Initializing | Stopping => ((74.0, 52.0), (54.0, 36.0)),
        Recording => ((186.0, 48.0), (170.0, 36.0)),
        Processing => ((120.0, 52.0), (100.0, 36.0)),
        ProcessingLong => ((202.0, 52.0), (184.0, 36.0)),
        Success => ((54.0, 52.0), (36.0, 36.0)),
        NoSpeech => ((254.0, 58.0), (236.0, 40.0)),
        Error => ((326.0, 58.0), (308.0, 46.0)),
    };

    Some(crate::models::GadgetHitRect {
        x: (window.0 - pill.0) / 2.0,
        y: (window.1 - pill.1) / 2.0,
        width: pill.0,
        height: pill.1,
    })
}

/// The native footprint is state-derived and stays close to the painted pill.
/// The small extra inset is only for the real shadow, not a permanent hitbox.
fn gadget_geometry(state: GadgetVisualState) -> GadgetGeometry {
    match state {
        GadgetVisualState::Hidden => GadgetGeometry {
            width: 1.0,
            height: 1.0,
        },
        GadgetVisualState::Idle | GadgetVisualState::Appearing => GadgetGeometry {
            width: 72.0,
            height: 52.0,
        },
        GadgetVisualState::Hover => GadgetGeometry {
            width: 142.0,
            height: 58.0,
        },
        GadgetVisualState::Initializing | GadgetVisualState::Stopping => GadgetGeometry {
            width: 74.0,
            height: 52.0,
        },
        GadgetVisualState::Recording => GadgetGeometry {
            width: 186.0,
            height: 48.0,
        },
        GadgetVisualState::Processing => GadgetGeometry {
            width: 120.0,
            height: 52.0,
        },
        GadgetVisualState::ProcessingLong => GadgetGeometry {
            width: 202.0,
            height: 52.0,
        },
        GadgetVisualState::Success => GadgetGeometry {
            width: 54.0,
            height: 52.0,
        },
        GadgetVisualState::Error => GadgetGeometry {
            width: 326.0,
            height: 58.0,
        },
        GadgetVisualState::NoSpeech => GadgetGeometry {
            width: 254.0,
            height: 58.0,
        },
    }
}

fn bottom_center_placement(
    anchor: &GadgetSessionAnchor,
    geometry: GadgetGeometry,
) -> (i32, i32, u32, u32) {
    let width = (geometry.width * anchor.scale).round().max(1.0) as u32;
    let height = (geometry.height * anchor.scale).round().max(1.0) as u32;
    let x = anchor.work_x as i64 + (anchor.work_width as i64 - width as i64) / 2;
    let y = anchor.work_y as i64 + anchor.work_height as i64
        - height as i64
        - (GADGET_BOTTOM_MARGIN * anchor.scale).round() as i64;
    (x as i32, y as i32, width, height)
}

fn monitor_anchor(monitor: &tauri::Monitor) -> GadgetSessionAnchor {
    let work = monitor.work_area();
    GadgetSessionAnchor {
        display_name: monitor.name().map(ToOwned::to_owned),
        work_x: work.position.x,
        work_y: work.position.y,
        work_width: work.size.width,
        work_height: work.size.height,
        scale: monitor.scale_factor(),
    }
}

fn select_gadget_anchor(
    configured: Option<GadgetSessionAnchor>,
    foreground: Option<GadgetSessionAnchor>,
    cursor: Option<GadgetSessionAnchor>,
    primary: Option<GadgetSessionAnchor>,
) -> Option<GadgetSessionAnchor> {
    foreground.or(configured).or(cursor).or(primary)
}

fn gadget_tracks_foreground_monitor(state: GadgetVisualState) -> bool {
    state != GadgetVisualState::Hidden
}

fn rectangles_overlap(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    a.0 < a.2
        && a.1 < a.3
        && b.0 < b.2
        && b.1 < b.3
        && a.0 < b.2
        && a.2 > b.0
        && a.1 < b.3
        && a.3 > b.1
}

fn choose_gadget_anchor(app: &tauri::AppHandle) -> Option<GadgetSessionAnchor> {
    let monitors = match app.available_monitors() {
        Ok(monitors) => monitors,
        Err(error) => {
            log::warn!("gadget: failed to enumerate displays: {}", error);
            Vec::new()
        }
    };

    let configured = crate::settings::load_widget_display().and_then(|saved_name| {
        let selected = monitors
            .iter()
            .find(|monitor| monitor.name().map(String::as_str) == Some(saved_name.as_str()))
            .map(monitor_anchor);
        if selected.is_none() {
            log::warn!(
                "gadget: persisted display '{}' is unavailable; using foreground/cursor/primary fallback",
                saved_name
            );
        }
        selected
    });

    #[cfg(target_os = "windows")]
    let foreground = win32_foreground_monitor_anchor();
    #[cfg(not(target_os = "windows"))]
    let foreground = None;

    let cursor = app.cursor_position().ok().and_then(|cursor| {
        monitors
            .iter()
            .find(|monitor| {
                let position = monitor.position();
                let size = monitor.size();
                cursor.x >= position.x as f64
                    && cursor.x < (position.x as f64 + size.width as f64)
                    && cursor.y >= position.y as f64
                    && cursor.y < (position.y as f64 + size.height as f64)
            })
            .map(monitor_anchor)
    });

    let primary = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|monitor| monitor_anchor(&monitor));

    select_gadget_anchor(configured, foreground, cursor, primary)
}

#[cfg(target_os = "windows")]
fn redraw_gadget_surface(window: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        RedrawWindow, RDW_ALLCHILDREN, RDW_INTERNALPAINT, RDW_INVALIDATE, RDW_UPDATENOW,
    };

    let Ok(tauri_hwnd) = window.hwnd() else {
        return;
    };
    // Tauri and the app intentionally resolve different windows-core patch
    // lines. Rebuild the app dependency's HWND from the stable raw handle.
    let hwnd = HWND(tauri_hwnd.0 as *mut _);
    // SAFETY: the HWND comes from the live Tauri window. RedrawWindow only
    // invalidates and synchronously paints this window and its WebView child.
    unsafe {
        let redrawn = RedrawWindow(
            hwnd,
            None,
            None,
            RDW_INVALIDATE | RDW_INTERNALPAINT | RDW_UPDATENOW | RDW_ALLCHILDREN,
        );
        if !redrawn.as_bool() {
            log::debug!("gadget: RedrawWindow did not schedule a paint");
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn redraw_gadget_surface(_window: &tauri::WebviewWindow) {}

/// Reinsert the gadget at the front of the Windows topmost band without
/// activating it. `set_always_on_top(true)` preserves `WS_EX_TOPMOST`, but is
/// effectively a no-op once that style is already set; other windows can then
/// remain ahead of the gadget in the Z-order and cover it until they are
/// minimized. An explicit `SetWindowPos(HWND_TOPMOST)` promotes the existing
/// HWND again on every visible presentation while `SWP_NOACTIVATE` preserves
/// focus in the application receiving dictation.
#[cfg(target_os = "windows")]
fn raise_gadget_topmost(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    let tauri_hwnd = window.hwnd().map_err(|error| error.to_string())?;
    let hwnd = HWND(tauri_hwnd.0 as *mut _);

    // SAFETY: the HWND belongs to the live gadget window. The flags prohibit
    // geometry and activation changes; only visibility and Z-order are
    // reaffirmed.
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    }
    .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "windows"))]
fn raise_gadget_topmost(_window: &tauri::WebviewWindow) -> Result<(), String> {
    Ok(())
}

/// Returns true when a visible, non-minimized top-level window ahead of the
/// gadget intersects its native footprint. The probe is entirely read-only and
/// can run off the Tauri event loop.
#[cfg(target_os = "windows")]
fn win32_gadget_is_obscured(hwnd_raw: isize) -> bool {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowRect, IsIconic, IsWindowVisible, GW_HWNDPREV,
    };

    if hwnd_raw == 0 {
        return false;
    }

    // SAFETY: every handle is either the cached live gadget HWND or a top-level
    // window returned by GetWindow. Failed/stale handles terminate or skip the
    // probe without mutating OS state.
    unsafe {
        let hwnd = HWND(hwnd_raw as *mut _);
        let mut gadget_rect = RECT::default();
        if GetWindowRect(hwnd, &mut gadget_rect).is_err() {
            return false;
        }
        let gadget_bounds = (
            gadget_rect.left,
            gadget_rect.top,
            gadget_rect.right,
            gadget_rect.bottom,
        );

        let mut above = match GetWindow(hwnd, GW_HWNDPREV) {
            Ok(window) => window,
            Err(_) => return false,
        };
        while !above.0.is_null() {
            if IsWindowVisible(above).as_bool() && !IsIconic(above).as_bool() {
                let mut rect = RECT::default();
                if GetWindowRect(above, &mut rect).is_ok()
                    && rectangles_overlap(
                        gadget_bounds,
                        (rect.left, rect.top, rect.right, rect.bottom),
                    )
                {
                    return true;
                }
            }
            above = match GetWindow(above, GW_HWNDPREV) {
                Ok(window) => window,
                Err(_) => break,
            };
        }
    }
    false
}

/// Keeps the visible gadget ahead of every overlapping desktop application.
/// The worker only queues a main-thread mutation when a read-only Win32 probe
/// detects a real obstruction, avoiding unconditional SetWindowPos traffic.
#[cfg(target_os = "windows")]
fn spawn_gadget_z_order_guardian(app: tauri::AppHandle, state: SharedState) {
    let promotion_pending = Arc::new(AtomicBool::new(false));
    std::thread::Builder::new()
        .name("sonora-gadget-z-order".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if *state.gadget_visual_state.read() == GadgetVisualState::Hidden {
                continue;
            }
            let hwnd_raw = GADGET_HWND.load(Ordering::Acquire);
            if !win32_gadget_is_obscured(hwnd_raw) || promotion_pending.swap(true, Ordering::AcqRel)
            {
                continue;
            }

            let app_for_promotion = app.clone();
            let state_for_promotion = state.clone();
            let pending_for_promotion = promotion_pending.clone();
            let pending_for_error = promotion_pending.clone();
            let queued = app.run_on_main_thread(move || {
                if *state_for_promotion.gadget_visual_state.read() != GadgetVisualState::Hidden {
                    if let Some(window) = app_for_promotion.get_webview_window("gadget") {
                        if let Err(error) = raise_gadget_topmost(&window) {
                            log::warn!("gadget: z-order promotion failed: {}", error);
                        } else {
                            cache_gadget_hwnd(&window);
                            log::debug!("gadget: promoted above an overlapping window");
                        }
                    }
                }
                pending_for_promotion.store(false, Ordering::Release);
            });
            if queued.is_err() {
                pending_for_error.store(false, Ordering::Release);
            }
        })
        .expect("failed to spawn gadget Z-order guardian");
}

#[cfg(not(target_os = "windows"))]
fn spawn_gadget_z_order_guardian(_app: tauri::AppHandle, _state: SharedState) {}

fn refresh_gadget_surface(window: &tauri::WebviewWindow) -> Result<(), String> {
    window.show().map_err(|error| error.to_string())?;
    window
        .set_always_on_top(true)
        .map_err(|error| error.to_string())?;
    raise_gadget_topmost(window)?;
    cache_gadget_hwnd(window);
    redraw_gadget_surface(window);
    Ok(())
}

fn gadget_render_is_pending(state: &SharedState, presentation: GadgetPresentation) -> bool {
    state.gadget_presentation_generation.load(Ordering::Acquire) == presentation.generation
        && state.gadget_rendered_generation.load(Ordering::Acquire) < presentation.generation
        && *state.gadget_visual_state.read() == presentation.visual_state
        && presentation.visual_state != GadgetVisualState::Hidden
}

fn spawn_gadget_render_watchdog(
    app: tauri::AppHandle,
    state: SharedState,
    presentation: GadgetPresentation,
) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(650));
        if !gadget_render_is_pending(&state, presentation) {
            return;
        }

        log::warn!(
            "gadget: render acknowledgement overdue state={:?} generation={}; forcing repaint",
            presentation.visual_state,
            presentation.generation
        );
        let app_for_repaint = app.clone();
        let state_for_repaint = state.clone();
        let queued = app.run_on_main_thread(move || {
            if !gadget_render_is_pending(&state_for_repaint, presentation) {
                return;
            }
            if let Some(window) = app_for_repaint.get_webview_window("gadget") {
                if let Err(error) = refresh_gadget_surface(&window) {
                    log::warn!("gadget: watchdog repaint failed: {}", error);
                }
                let _ = window.eval("window.dispatchEvent(new Event('sonora-gadget-repaint'))");
            }
        });
        if let Err(error) = queued {
            log::warn!("gadget: failed to queue watchdog repaint: {}", error);
        }

        std::thread::sleep(std::time::Duration::from_millis(1_350));
        if !gadget_render_is_pending(&state, presentation) {
            return;
        }

        log::error!(
            "gadget: render acknowledgement missing state={:?} generation={}; reloading overlay",
            presentation.visual_state,
            presentation.generation
        );
        let app_for_reload = app.clone();
        let state_for_reload = state.clone();
        let queued = app.run_on_main_thread(move || {
            if !gadget_render_is_pending(&state_for_reload, presentation) {
                return;
            }
            if let Some(window) = app_for_reload.get_webview_window("gadget") {
                if let Err(error) = window.reload() {
                    log::error!("gadget: watchdog reload failed: {}", error);
                }
            }
        });
        if let Err(error) = queued {
            log::warn!("gadget: failed to queue watchdog reload: {}", error);
        }
    });
}

pub(crate) fn acknowledge_gadget_rendered(
    app: &tauri::AppHandle,
    state: &SharedState,
    presentation: GadgetPresentation,
    rect: crate::models::GadgetHitRect,
) -> Result<bool, String> {
    let valid_rect = [rect.x, rect.y, rect.width, rect.height]
        .into_iter()
        .all(f64::is_finite)
        && rect.width > 0.0
        && rect.height > 0.0;
    if !valid_rect {
        return Err("gadget reported an invalid render rectangle".to_string());
    }

    if state.gadget_presentation_generation.load(Ordering::Acquire) != presentation.generation
        || *state.gadget_visual_state.read() != presentation.visual_state
        || presentation.visual_state == GadgetVisualState::Hidden
    {
        log::debug!(
            "gadget: ignored stale render acknowledgement state={:?} generation={}",
            presentation.visual_state,
            presentation.generation
        );
        return Ok(false);
    }

    *state.gadget_hit_rect.write() = Some(rect);
    state
        .gadget_rendered_generation
        .store(presentation.generation, Ordering::Release);
    let window = app
        .get_webview_window("gadget")
        .ok_or_else(|| "gadget window is unavailable".to_string())?;
    refresh_gadget_surface(&window)?;
    log::info!(
        "gadget: rendered state={:?} generation={} rect=({:.1},{:.1} {:.1}x{:.1})",
        presentation.visual_state,
        presentation.generation,
        rect.x,
        rect.y,
        rect.width,
        rect.height
    );
    Ok(true)
}

fn apply_gadget_presentation(
    app: &tauri::AppHandle,
    state: &SharedState,
    requested: GadgetVisualState,
) -> Result<GadgetPresentation, String> {
    let resolved = if requested == GadgetVisualState::Hidden
        && *state.widget_visibility_mode.read() == WidgetVisibilityMode::Always
    {
        GadgetVisualState::Idle
    } else {
        requested
    };
    let previous = {
        let mut current = state.gadget_visual_state.write();
        let previous = *current;
        *current = resolved;
        previous
    };
    let generation = if previous != resolved {
        state
            .gadget_presentation_generation
            .fetch_add(1, Ordering::AcqRel)
            + 1
    } else {
        state.gadget_presentation_generation.load(Ordering::Acquire)
    };
    let presentation = GadgetPresentation {
        visual_state: resolved,
        generation,
    };

    let Some(window) = app.get_webview_window("gadget") else {
        return Err("gadget window is unavailable".to_string());
    };

    if resolved == GadgetVisualState::Hidden {
        let geometry = gadget_geometry(resolved);
        window
            .set_size(tauri::PhysicalSize::new(
                geometry.width as u32,
                geometry.height as u32,
            ))
            .map_err(|error| error.to_string())?;
        let _ = window.set_ignore_cursor_events(true);
        // Keep the native window and WebView controller attached for the full
        // app lifetime. Repeated hide/show cycles can leave WebView2 alive but
        // no longer presenting frames after long-running background use.
        window.show().map_err(|error| error.to_string())?;
        *state.gadget_hit_rect.write() = None;
        *state.gadget_session_anchor.lock() = None;
        log::info!(
            "gadget: state=Hidden generation={} surface=attached",
            generation
        );
        return Ok(presentation);
    }

    let anchor = {
        let mut current = state.gadget_session_anchor.lock();
        if current.is_none() {
            *current = choose_gadget_anchor(app);
        }
        current.clone()
    }
    .ok_or_else(|| "no display is available for the gadget".to_string())?;

    let geometry = gadget_geometry(resolved);
    let (x, y, physical_width, physical_height) = bottom_center_placement(&anchor, geometry);

    // Keep a native fallback in sync with the CSS pill geometry. React reports
    // the measured rectangle too, but focus-monitor moves and fast state
    // transitions can happen without a new ResizeObserver notification.
    // Never force click-through here: doing so desynchronizes the watcher when
    // the cursor was already inside the pill and makes its buttons unclickable.
    *state.gadget_hit_rect.write() = gadget_hit_rect(resolved);
    window
        .set_size(tauri::PhysicalSize::new(physical_width, physical_height))
        .map_err(|error| error.to_string())?;
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|error| error.to_string())?;
    refresh_gadget_surface(&window)?;
    log::info!(
        "gadget: state={:?} generation={} display={:?} work=({},{} {}x{}) window=({},{} {}x{}) scale={:.2}",
        resolved,
        generation,
        anchor.display_name,
        anchor.work_x,
        anchor.work_y,
        anchor.work_width,
        anchor.work_height,
        x,
        y,
        physical_width,
        physical_height,
        anchor.scale,
    );
    if previous != resolved {
        spawn_gadget_render_watchdog(app.clone(), state.clone(), presentation);
    }
    Ok(presentation)
}

pub(crate) fn begin_gadget_session(app: &tauri::AppHandle, state: &SharedState) {
    let mut anchor = state.gadget_session_anchor.lock();
    *anchor = choose_gadget_anchor(app);
    drop(anchor);
    if let Err(error) = apply_gadget_presentation(app, state, GadgetVisualState::Appearing) {
        log::warn!("gadget: failed to begin session: {}", error);
    }
}

pub(crate) fn present_gadget(
    app: &tauri::AppHandle,
    state: &SharedState,
    visual_state: GadgetVisualState,
) -> Result<GadgetPresentation, String> {
    apply_gadget_presentation(app, state, visual_state)
}

/// Creates the always-on-top floating gadget overlay. It loads the same bundle
/// as the main window but is told apart by its `"gadget"` label (the frontend
/// renders a different root for it). The window is transparent, frameless,
/// hidden from the taskbar and always on top. It stays natively visible for the
/// whole process lifetime; Auto mode collapses it to a transparent, click-
/// through 1x1 surface instead of cycling WebView2 through hide/show.
///
/// Position persistence uses **physical** pixels (relative to the virtual
/// desktop origin) rather than logical pixels. This is the crucial fix for
/// multi-monitor setups: logical coordinates depend on the scale factor of
/// the monitor the window is being created on, which is the *primary*
/// monitor at creation time — so a position saved on a 1.5x DPI secondary
/// monitor would be reinterpreted against the 1.0x primary monitor and land
/// in the wrong spot. Physical pixels are scale-independent and therefore
/// round-trip reliably across any monitor configuration.
fn setup_gadget(app: &tauri::AppHandle, state: &SharedState) {
    let initial = if *state.widget_visibility_mode.read() == WidgetVisibilityMode::Always {
        GadgetVisualState::Idle
    } else {
        GadgetVisualState::Hidden
    };
    let geometry = gadget_geometry(initial);
    let builder = WebviewWindowBuilder::new(app, "gadget", WebviewUrl::App("index.html".into()))
        .title("Sonora Dictation Bar")
        .inner_size(geometry.width, geometry.height)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .visible(true)
        .focused(false)
        .focusable(false);

    match builder.build() {
        Ok(window) => {
            cache_gadget_hwnd(&window);
            if let Err(error) = apply_gadget_presentation(app, state, initial) {
                log::warn!("gadget: initial presentation failed: {}", error);
            }
        }
        Err(e) => log::error!("gadget: failed to create overlay window: {}", e),
    }
}

/// Win32 hit-test: is the cursor over the gadget's visible pill?
///
/// Uses only thread-safe Win32 reads (`GetCursorPos`, `GetWindowRect`,
/// `GetDpiForWindow`) so this never touches Tauri's event loop. The pill rect
/// lives in logical pixels relative to the window origin and is scaled by the
/// window DPI. Until the frontend reports a rect, visible states return `true`
/// (fully interactive — fail-safe); Hidden always remains click-through.
#[cfg(target_os = "windows")]
fn win32_cursor_inside_pill(hwnd_raw: isize, state: &SharedState) -> bool {
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetWindowRect, IsWindow, IsWindowVisible,
    };

    if hwnd_raw == 0 {
        return true;
    }
    if *state.gadget_visual_state.read() == GadgetVisualState::Hidden {
        return false;
    }
    let hwnd = HWND(hwnd_raw as *mut _);

    // SAFETY: HWND was captured from a live WebviewWindow; IsWindow validates
    // it before any further use. These Win32 getters are documented as safe to
    // call from any thread.
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            GADGET_HWND.store(0, Ordering::Release);
            return true;
        }
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return true;
        }
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            return true;
        }

        let dpi = GetDpiForWindow(hwnd);
        let scale = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };

        match *state.gadget_hit_rect.read() {
            // Fail interactive while the frontend/native fallback is not yet
            // available. A visible gadget must never become permanently
            // click-through because one rect update was missed.
            None => true,
            Some(r) => {
                let left = rect.left as f64 + r.x * scale;
                let top = rect.top as f64 + r.y * scale;
                let right = left + r.width * scale;
                let bottom = top + r.height * scale;
                let cx = pt.x as f64;
                let cy = pt.y as f64;
                cx >= left && cx <= right && cy >= top && cy <= bottom
            }
        }
    }
}

/// Spawns a lightweight background thread that keeps the gadget overlay
/// click-through everywhere except over its visible pill.
///
/// ## Why this design (AppHangB1 / "Não respondendo")
///
/// Earlier versions either:
/// 1. Called `set_ignore_cursor_events` from a worker (tao marshals
///    `SetWindowPos(SWP_FRAMECHANGED)` via synchronous `SendMessage` into the
///    WNDPROC on the main thread → stall when Windows throttles a background
///    overlay, e.g. when Task Manager steals focus), or
/// 2. Queued a full hit-test on the main thread every 100 ms (still hammers the
///    single shared event loop with HWND reads under focus storms).
///
/// Current approach:
/// - **Hit-test on the worker** with pure Win32 APIs + cached HWND (no Tauri
///   window calls per tick).
/// - **Main-thread work only when the ignore flag actually changes**, and at
///   most one pending toggle at a time.
/// - Cadence ~32 ms; hit-testing stays on the worker and main-thread work still
///   happens only at cursor-boundary changes.
///
/// Native state supplies an immediate fallback rect; React measurements refine
/// it after rendering.
fn spawn_gadget_cursor_watcher_safe(app: tauri::AppHandle, state: SharedState) {
    let pending_toggle = Arc::new(AtomicBool::new(false));

    std::thread::spawn(move || {
        let mut last_ignore: Option<bool> = None;
        let mut tick: u32 = 0;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(32));
            tick = tick.wrapping_add(1);

            #[cfg(target_os = "windows")]
            {
                let hwnd_raw = GADGET_HWND.load(Ordering::Acquire);

                // Rare HWND re-resolve if cache is empty (window recreated, etc.).
                // Coalesced with the toggle pending flag so we never flood main.
                if hwnd_raw == 0
                    && tick.is_multiple_of(5)
                    && !pending_toggle.load(Ordering::Acquire)
                {
                    let app_resolve = app.clone();
                    let pending = pending_toggle.clone();
                    if pending.swap(true, Ordering::AcqRel) {
                        continue;
                    }
                    let queued = app.run_on_main_thread(move || {
                        if let Some(w) = app_resolve.get_webview_window("gadget") {
                            cache_gadget_hwnd(&w);
                        }
                        pending.store(false, Ordering::Release);
                    });
                    if queued.is_err() {
                        pending_toggle.store(false, Ordering::Release);
                    }
                    continue;
                }

                if hwnd_raw == 0 {
                    continue;
                }

                let inside = win32_cursor_inside_pill(hwnd_raw, &state);
                // If HWND was invalidated during hit-test, skip this tick.
                if GADGET_HWND.load(Ordering::Acquire) == 0 {
                    last_ignore = None;
                    continue;
                }

                let want_ignore = !inside;
                if last_ignore == Some(want_ignore) {
                    continue;
                }

                // At most one pending HWND mutation on the event loop.
                if pending_toggle.swap(true, Ordering::AcqRel) {
                    continue;
                }

                last_ignore = Some(want_ignore);
                log::info!(
                    "gadget: cursor watcher toggle (ignore={} inside={})",
                    want_ignore,
                    inside
                );

                let app_for_toggle = app.clone();
                let pending_for_toggle = pending_toggle.clone();
                let pending_for_error = pending_toggle.clone();
                let queued = app.run_on_main_thread(move || {
                    if let Some(w) = app_for_toggle.get_webview_window("gadget") {
                        cache_gadget_hwnd(&w);
                        let _ = w.set_ignore_cursor_events(want_ignore);
                    }
                    pending_for_toggle.store(false, Ordering::Release);
                });
                if queued.is_err() {
                    pending_for_error.store(false, Ordering::Release);
                    // Allow a re-apply on the next edge if the queue failed.
                    last_ignore = None;
                }
            }

            #[cfg(not(target_os = "windows"))]
            {
                // Non-Windows fallback: infrequent main-thread evaluation only
                // when needed (same coalescing rules).
                if pending_toggle.swap(true, Ordering::AcqRel) {
                    continue;
                }
                let app_for_tick = app.clone();
                let state_for_tick = state.clone();
                let pending_for_tick = pending_toggle.clone();
                let pending_for_error = pending_toggle.clone();
                let last_snapshot = last_ignore;
                let queued = app.run_on_main_thread(move || {
                    let Some(window) = app_for_tick.get_webview_window("gadget") else {
                        pending_for_tick.store(false, Ordering::Release);
                        return;
                    };
                    if !window.is_visible().unwrap_or(false) {
                        pending_for_tick.store(false, Ordering::Release);
                        return;
                    }
                    let inside = match *state_for_tick.gadget_hit_rect.read() {
                        None => false,
                        Some(r) => {
                            match (app_for_tick.cursor_position(), window.outer_position()) {
                                (Ok(cursor), Ok(pos)) => {
                                    let scale = window.scale_factor().unwrap_or(1.0);
                                    let left = pos.x as f64 + r.x * scale;
                                    let top = pos.y as f64 + r.y * scale;
                                    let right = left + r.width * scale;
                                    let bottom = top + r.height * scale;
                                    cursor.x >= left
                                        && cursor.x <= right
                                        && cursor.y >= top
                                        && cursor.y <= bottom
                                }
                                _ => true,
                            }
                        }
                    };
                    let want_ignore = !inside;
                    if last_snapshot != Some(want_ignore) {
                        log::info!(
                            "gadget: cursor watcher toggle (ignore={} inside={})",
                            want_ignore,
                            inside
                        );
                        let _ = window.set_ignore_cursor_events(want_ignore);
                    }
                    pending_for_tick.store(false, Ordering::Release);
                });
                if queued.is_err() {
                    pending_for_error.store(false, Ordering::Release);
                }
                // Best-effort local tracking; next tick may re-apply.
                if last_ignore.is_none() {
                    last_ignore = Some(false);
                }
            }
        }
    });
}

#[cfg(target_os = "windows")]
fn win32_foreground_monitor_anchor() -> Option<GadgetSessionAnchor> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    // SAFETY: all calls are read-only OS queries. The foreground HWND and
    // monitor handle are validated by their documented null results.
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() || foreground.0 as isize == GADGET_HWND.load(Ordering::Acquire) {
            return None;
        }
        let monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST);
        if monitor.0.is_null() {
            return None;
        }
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
            return None;
        }
        let dpi = GetDpiForWindow(foreground);
        let work = info.monitorInfo.rcWork;
        let display_name_length = info
            .szDevice
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(info.szDevice.len());
        let display_name = String::from_utf16_lossy(&info.szDevice[..display_name_length]);
        Some(GadgetSessionAnchor {
            display_name: (!display_name.is_empty()).then_some(display_name),
            work_x: work.left,
            work_y: work.top,
            work_width: (work.right - work.left).max(1) as u32,
            work_height: (work.bottom - work.top).max(1) as u32,
            scale: if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 },
        })
    }
}

#[cfg(target_os = "windows")]
fn spawn_gadget_focus_monitor_watcher(app: tauri::AppHandle, state: SharedState) {
    let pending_move = Arc::new(AtomicBool::new(false));
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let Some(next_anchor) = win32_foreground_monitor_anchor() else {
            continue;
        };
        let current_state = *state.gadget_visual_state.read();
        if !gadget_tracks_foreground_monitor(current_state) {
            continue;
        }
        let unchanged = state
            .gadget_session_anchor
            .lock()
            .as_ref()
            .map(|current| {
                current.work_x == next_anchor.work_x
                    && current.work_y == next_anchor.work_y
                    && current.work_width == next_anchor.work_width
                    && current.work_height == next_anchor.work_height
                    && (current.scale - next_anchor.scale).abs() < 0.01
            })
            .unwrap_or(false);
        if unchanged || pending_move.swap(true, Ordering::AcqRel) {
            continue;
        }

        let app_for_move = app.clone();
        let state_for_move = state.clone();
        let pending_for_move = pending_move.clone();
        let pending_for_error = pending_move.clone();
        let queued = app.run_on_main_thread(move || {
            let current_state = *state_for_move.gadget_visual_state.read();
            if gadget_tracks_foreground_monitor(current_state) {
                *state_for_move.gadget_session_anchor.lock() = Some(next_anchor);
                if let Err(error) =
                    apply_gadget_presentation(&app_for_move, &state_for_move, current_state)
                {
                    log::warn!("gadget: focus-monitor move failed: {}", error);
                }
            }
            pending_for_move.store(false, Ordering::Release);
        });
        if queued.is_err() {
            pending_for_error.store(false, Ordering::Release);
        }
    });
}

#[cfg(not(target_os = "windows"))]
fn spawn_gadget_focus_monitor_watcher(_app: tauri::AppHandle, _state: SharedState) {}

/// Builds the system tray icon and its context menu. The app keeps running in
/// the background (window hidden) when closed, and the tray lets the user
/// restore the window or quit for good. Returns the built tray so it stays
/// alive for the lifetime of the app.
fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Mostrar Sonora", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    let mut builder = TrayIconBuilder::with_id("sonora-tray")
        .tooltip("Sonora")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => restore_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // A left click on the tray icon restores the main window.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                restore_main_window(tray.app_handle());
            }
        });

    // Reuse the app's window icon for the tray when one is embedded.
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

/// Shows and focuses the main window, recreating visibility after a
/// close-to-tray. Safe to call when the window is already visible.
fn restore_main_window(app: &tauri::AppHandle) {
    log::info!("window: restore_main_window requested");
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Returns the per-user log directory for Sonora. Computed from the
/// `APPDATA` environment variable and the Tauri bundle identifier so it is
/// available **before** the Tauri runtime initialises (the `app.path()`
/// resolver is not yet available at that point).
///
/// On Windows: `%APPDATA%\com.haumeavoice.app\logs\`
fn logs_dir() -> Option<std::path::PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    let dir = std::path::PathBuf::from(appdata)
        .join("com.haumeavoice.app")
        .join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Writer that sends every byte to both a file and stderr. Passed to
/// `env_logger` via `Target::Pipe` so runtime log lines are persisted to
/// `app.log` while still appearing in the console during `cargo tauri dev`.
struct TeeWriter {
    file: std::fs::File,
    path: std::path::PathBuf,
}

impl std::io::Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // stderr first (best-effort; never fail the whole write for it)
        let _ = std::io::stderr().write_all(buf);
        if self.file.metadata()?.len() + buf.len() as u64 > 8 * 1024 * 1024 {
            if let Some(dir) = self.path.parent() {
                for index in (1..4).rev() {
                    let source = dir.join(format!("app.{index}.log"));
                    if source.exists() {
                        std::fs::rename(source, dir.join(format!("app.{}.log", index + 1)))?;
                    }
                }
                std::fs::rename(&self.path, dir.join("app.1.log"))?;
                self.file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?;
            }
        }
        self.file.write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        self.file.flush()
    }
}

/// Initializes logging. In production builds the output is tee'd to
/// `%APPDATA%/com.haumeavoice.app/logs/app.log` (rotated, 8 MiB per file)
/// so runtime messages survive for post-mortem inspection. In dev mode
/// (or if the file cannot be opened) output goes only to stderr.
fn init_logging() {
    let mut builder = env_logger::Builder::from_default_env();
    builder
        .filter_level(log::LevelFilter::Info)
        .format_timestamp_secs();

    if let Some(dir) = logs_dir() {
        let log_path = dir.join("app.log");
        // Rotate between sessions; preserve a bounded set of previous logs.
        if log_path.exists() {
            for index in (1..4).rev() {
                let source = dir.join(format!("app.{index}.log"));
                if source.exists() {
                    let _ = std::fs::rename(source, dir.join(format!("app.{}.log", index + 1)));
                }
            }
            let _ = std::fs::rename(&log_path, dir.join("app.1.log"));
        }
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            builder.target(env_logger::Target::Pipe(Box::new(TeeWriter {
                file,
                path: log_path,
            })));
        }
    }

    builder.format(|buffer, record| {
        use std::io::Write;
        writeln!(
            buffer,
            "{} [{}] {}",
            buffer.timestamp_seconds(),
            record.level(),
            crate::redaction::message(&record.args().to_string())
        )
    });
    let _ = builder.try_init();
}

/// Installs a Windows vectored exception handler that dumps any *native*
/// (non-Rust-panic) fatal exception — access violation, stack overflow,
/// illegal instruction, etc. — to `crash.log` before the process is torn
/// down. Rust's `std::panic` hook (set in [`run`]) only catches panics; a
/// native crash otherwise leaves no in-app forensic trail (only a Windows
/// WER report, which is hard to read). The handler returns
/// `EXCEPTION_CONTINUE_SEARCH` so the default OS handler still runs and
/// produces its WER report — we just log first.
#[cfg(target_os = "windows")]
fn install_native_crash_handler() {
    use windows::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_POINTERS,
    };

    unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
        // EXCEPTION_CONTINUE_SEARCH (0): let the default handler run after us.
        const CONTINUE_SEARCH: i32 = 0;
        if info.is_null() {
            return CONTINUE_SEARCH;
        }
        // SAFETY: the OS guarantees `info` is valid for the duration of the
        // handler call. We only read from it.
        let pointers = unsafe { &*info };
        let rec_ptr = pointers.ExceptionRecord;
        if rec_ptr.is_null() {
            return CONTINUE_SEARCH;
        }
        let record = unsafe { &*rec_ptr };
        // Use Debug repr for the code/NTSTATUS and `:p` for the address so we
        // never depend on the exact ABI types. Best-effort file write —
        // allocating inside an exception handler can deadlock if the exception
        // happened in the heap, but for the common access-violation / stack-
        // overflow cases this completes and is invaluable for diagnosis.
        let code = format!("{:?}", record.ExceptionCode);
        let addr = format!("{:p}", record.ExceptionAddress);
        let msg = format!(
            "[NATIVE EXCEPTION] code={} addr={}\nBacktrace: <not captured; see Windows WER report>\n",
            code, addr
        );
        log::error!("{}", msg.trim());
        if let Some(dir) = logs_dir() {
            let log_path = dir.join("crash.log");
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                use std::io::Write;
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let _ = writeln!(f, "--- {} ---\n{}", ts, msg);
            }
        }
        CONTINUE_SEARCH
    }

    // first = 0 → our handler runs after earlier handlers, effectively for
    // every unhandled exception. The returned handle is intentionally never
    // freed: it must live for the whole process lifetime.
    let _ = unsafe { AddVectoredExceptionHandler(0, Some(handler)) };
    log::info!("crash: native vectored exception handler installed");
}

/// No-op on non-Windows platforms.
#[cfg(not(target_os = "windows"))]
fn install_native_crash_handler() {}

/// Heartbeat watchdog that proves whether Tauri's single shared event-loop
/// thread (the "main thread") is still pumping.
///
/// A background thread schedules a closure back onto the main thread every
/// 2 s via [`tauri::AppHandle::run_on_main_thread`]; the closure bumps a
/// monotonic counter. Soft stalls (≥2 s) are logged to `app.log` only.
/// **Serious** stalls (≥6 s, aligned with Windows AppHangB1) are also
/// appended to `crash.log` with context (recording flag, gadget HWND).
///
/// This is the decisive diagnostic for the recurring AppHangB1
/// ("Não respondendo"): when Windows marks the process Not Responding, the
/// last STALLED line in `crash.log` confirms the main thread was blocked.
fn spawn_main_thread_heartbeat(app: tauri::AppHandle) {
    static HEARTBEAT_TICK: AtomicI64 = AtomicI64::new(0);
    std::thread::spawn(move || {
        let mut stall_start: Option<std::time::Instant> = None;
        let mut wrote_crash_log = false;
        loop {
            // Schedule a tick on the main thread. If the main thread is
            // blocked, this closure simply won't run — which is exactly what
            // the counter comparison below detects.
            let before_tick = HEARTBEAT_TICK.load(Ordering::Relaxed);
            let _ = app.run_on_main_thread(move || {
                HEARTBEAT_TICK.fetch_add(1, Ordering::Relaxed);
            });
            std::thread::sleep(std::time::Duration::from_secs(2));
            let now_tick = HEARTBEAT_TICK.load(Ordering::Relaxed);
            if now_tick == before_tick {
                // Main thread hasn't ticked since the last check (≥2 s).
                match stall_start {
                    None => {
                        stall_start = Some(std::time::Instant::now());
                        wrote_crash_log = false;
                        let recording = app
                            .try_state::<SharedState>()
                            .map(|s| s.is_recording())
                            .unwrap_or(false);
                        #[cfg(target_os = "windows")]
                        let hwnd = GADGET_HWND.load(Ordering::Relaxed);
                        #[cfg(not(target_os = "windows"))]
                        let hwnd = 0isize;
                        log::warn!(
                            "heartbeat: main-thread soft-stall (tick={}, recording={}, gadget_hwnd={:#x})",
                            now_tick, recording, hwnd
                        );
                    }
                    Some(start) => {
                        let elapsed = start.elapsed().as_secs();
                        if elapsed >= 6 && !wrote_crash_log {
                            wrote_crash_log = true;
                            let recording = app
                                .try_state::<SharedState>()
                                .map(|s| s.is_recording())
                                .unwrap_or(false);
                            #[cfg(target_os = "windows")]
                            let hwnd = GADGET_HWND.load(Ordering::Relaxed);
                            #[cfg(not(target_os = "windows"))]
                            let hwnd = 0isize;
                            log::error!(
                                "heartbeat: main thread STALLED {}s (tick={}, recording={}, gadget_hwnd={:#x})",
                                elapsed, now_tick, recording, hwnd
                            );
                            if let Some(dir) = logs_dir() {
                                let log_path = dir.join("crash.log");
                                if let Ok(mut f) = std::fs::OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(&log_path)
                                {
                                    use std::io::Write;
                                    let ts = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();
                                    let _ = writeln!(
                                        f,
                                        "--- {} ---\n[APPHANG DETECTED] main thread stalled {}s (tick={}, recording={}, gadget_hwnd={:#x})\n",
                                        ts, elapsed, now_tick, recording, hwnd
                                    );
                                }
                            }
                        } else if elapsed == 15 || elapsed == 30 || elapsed == 60 {
                            log::error!("heartbeat: main thread STILL stalled after {}s", elapsed);
                        }
                    }
                }
            } else if let Some(start) = stall_start.take() {
                let elapsed = start.elapsed().as_secs().min(120);
                let was_serious = wrote_crash_log;
                wrote_crash_log = false;
                log::warn!("heartbeat: main thread RESUMED after {}s stall", elapsed);
                // Only persist RESOLVED when we previously wrote a serious stall.
                if was_serious {
                    if let Some(dir) = logs_dir() {
                        let log_path = dir.join("crash.log");
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&log_path)
                        {
                            use std::io::Write;
                            let ts = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            let _ = writeln!(
                                f,
                                "--- {} ---\n[APPHANG RESOLVED] main thread resumed after {}s\n",
                                ts, elapsed
                            );
                        }
                    }
                }
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _instance = match single_instance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) => {
            eprintln!("Não foi possível iniciar: {error}");
            return;
        }
    };
    init_logging();
    log::info!("startup: run() entry");

    // In release GUI mode there is no console, so a panic message vanishes
    // silently. This hook writes the panic info and a backtrace to
    // `crash.log` inside the app's per-user log directory so any future
    // panic leaves a forensic trail that can be inspected post-mortem.
    std::panic::set_hook({
        let crash_dir = logs_dir();
        Box::new(move |info| {
            let bt = std::backtrace::Backtrace::force_capture();
            let msg = format!("[PANIC] {}\nBacktrace:\n{}\n", info, bt);
            // Also goes to app.log via the TeeWriter (best-effort).
            log::error!("{}", msg);

            if let Some(ref dir) = crash_dir {
                let log_path = dir.join("crash.log");
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
                {
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let _ = writeln!(f, "--- {} ---\n{}", timestamp, msg);
                }
            }
        })
    });

    install_native_crash_handler();
    log::info!("startup: beginning tauri builder setup");

    let state: SharedState = Arc::new(models::AppState::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state.clone())
        .setup({
            let state = state.clone();
            move |app| {
                log::info!("startup: setup begin");
                // Resolve the per-user app data directory once and hand it
                // to the history module so transcription snapshots can be
                // persisted to `history.json`.
                match app.path().app_data_dir() {
                    Ok(dir) => {
                        let history_file = dir.join("history.json");
                        history::init(history_file);
                        log::info!("history: storage at {:?}", dir);

                        // Wire up persistent API key storage and load any
                        // previously saved keys into the in-memory state so
                        // they are available immediately after a restart.
                        secrets::init(dir.join("api_keys.json"));
                        *state.api_keys.write() = secrets::load().unwrap_or_else(|error| {
                            log::error!("credentials: {error}");
                            crate::models::ApiKeys::default()
                        });
                        crate::redaction::register(&state.api_keys.read());
                        log::info!("secrets: protected storage initialized");

                        // Permanent storage for the audio behind each
                        // transcription (used later by the pronunciation
                        // evaluator). Lives in an `audio` sub-directory.
                        audio_store::init(dir.join("audio"));
                        log::info!("audio_store: storage at {:?}", dir.join("audio"));

                        // Load any persisted custom recording shortcuts into
                        // the state before they are registered below.
                        shortcuts::init_store(dir.join("shortcuts.json"));
                        *state.shortcuts.write() = shortcuts::load_store();

                        // Lightweight UI preferences (gadget compact mode).
                        settings::init(dir.join("settings.json"));
                        context::init(dir.clone());
                        scratchpad::init(dir.clone());
                        snippets::init(dir.clone());
                        learning::init(dir.clone());
                        local_whisper::init(dir.clone());
                        insights::init(dir.clone());
                        *state.compact_mode.write() = settings::load_compact();
                        *state.widget_visibility_mode.write() =
                            settings::load_widget_visibility_mode();
                        *state.widget_dock.write() = settings::load_widget_dock();
                        *state.system_prompt.write() = settings::load_system_prompt();

                        if let Some(engine) = settings::load_engine() {
                            *state.engine.write() = engine;
                        }
                        if let Some(sanitizer) = settings::load_sanitizer() {
                            *state.sanitizer.write() = sanitizer;
                        }
                        *state.dual_engine.write() = settings::load_dual_engine();
                        *state.reasoning_enabled.write() = settings::load_reasoning_enabled();
                        *state.deepgram_mode.write() = settings::load_deepgram_mode();
                        *state.sanitizer_enabled.write() = settings::load_sanitizer_enabled();
                        *state.reasoning_effort.write() = settings::load_reasoning_effort();
                        *state.vocabulary.write() = settings::load_vocabulary();
                        *state.modes_enabled.write() = settings::load_modes_enabled();
                        *state.transcription_mode.write() = settings::load_transcription_mode();
                        *state.gemini_fallback_to_whisper.write() =
                            settings::load_gemini_fallback_to_whisper();
                        *state.file_tagging_enabled.write() = settings::load_file_tagging_enabled();
                        *state.gemini_pipelines.write() = settings::load_gemini_pipelines();
                        *state.context_preferences.write() = settings::load_context_preferences();
                        *state.output_profiles.write() = settings::load_output_profiles();
                        *state.formatting_level.write() = settings::load_formatting_level();
                        *state.dictation_destination.write() =
                            settings::load_dictation_destination();
                        *state.features.write() = settings::load_features();
                        insights::start_backfill(app.handle().clone());
                    }
                    Err(e) => {
                        log::warn!("history: could not resolve app data dir: {}", e);
                    }
                }

                // Register the global recording shortcuts from the (possibly
                // persisted) config. A bad persisted combination is logged
                // rather than crashing startup.
                let cfg = state.shortcuts.read().clone();
                if let Err(e) = shortcuts::register_all(app.handle(), &cfg) {
                    log::error!("failed to register global shortcuts: {}", e);
                }

                // Hand the AppHandle to the audio pipeline so finished
                // transcriptions can be persisted and announced to the UI.
                *state.app_handle.write() = Some(app.handle().clone());

                // Build the system tray so the app can live in the background.
                if let Err(e) = setup_tray(app.handle()) {
                    log::error!("failed to create system tray: {}", e);
                }

                // Spawn the always-on-top floating gadget overlay.
                setup_gadget(app.handle(), &state);

                // Make the overlay click-through outside its visible pill so it
                // no longer eats clicks that land near (but not on) the gadget.
                spawn_gadget_cursor_watcher_safe(app.handle().clone(), state.clone());
                spawn_gadget_focus_monitor_watcher(app.handle().clone(), state.clone());
                updates::spawn(app.handle().clone(), state.clone());
                spawn_gadget_z_order_guardian(app.handle().clone(), state.clone());

                // If not launched via autostart, show the main window.
                let is_autostart = std::env::args().any(|arg| arg == "--autostart");
                if !is_autostart {
                    if let Some(w) = app.handle().get_webview_window("main") {
                        if let Ok(Some(monitor)) = w.current_monitor() {
                            let work = monitor.work_area();
                            let scale = monitor.scale_factor();
                            let width =
                                (work.size.width as f64 / scale - 32.0).clamp(480.0, 1400.0);
                            let height =
                                (work.size.height as f64 / scale - 32.0).clamp(320.0, 900.0);
                            let _ = w.set_min_size(Some(tauri::LogicalSize::new(
                                width.min(720.0),
                                height.min(480.0),
                            )));
                            let _ = w.set_size(tauri::LogicalSize::new(width, height));
                            let _ = w.center();
                        }
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }

                // Launch the main-thread heartbeat watchdog so any future
                // AppHangB1 leaves a definitive STALLED/RESUMED trail in both
                // app.log and crash.log.
                spawn_main_thread_heartbeat(app.handle().clone());

                log::info!("startup: setup complete");
                Ok(())
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::update_engine_config,
            commands::get_engine_config,
            commands::get_mode_config,
            commands::update_mode_config,
            commands::save_api_keys,
            commands::get_api_keys,
            commands::transcribe_file,
            commands::evaluate_pronunciation,
            commands::toggle_recording_state,
            commands::cancel_recording,
            commands::get_recording_status,
            commands::get_recording_elapsed,
            commands::get_local_diagnostics,
            commands::archive_history_audio,
            commands::retry_recovery_audio,
            commands::export_local_data,
            commands::import_local_data,
            commands::get_history_page,
            commands::get_history_detail,
            commands::restore_history_entry,
            commands::repair_history_journal,
            commands::get_audio_storage_config,
            commands::set_audio_storage_directory,
            commands::reveal_history_audio,
            commands::read_history_audio,
            commands::clear_history,
            commands::delete_history_entry,
            commands::update_history_text,
            commands::get_vocabulary,
            commands::set_vocabulary,
            commands::get_widget_preferences,
            commands::set_widget_visibility_mode,
            commands::set_gadget_visual_state,
            commands::acknowledge_gadget_rendered,
            commands::get_shortcuts,
            commands::set_shortcuts,
            commands::list_audio_devices,
            commands::get_input_device,
            commands::set_input_device,
            commands::start_mic_test,
            commands::stop_mic_test,
            commands::retry_transcription,
            commands::retry_transcription_with_fallback,
            commands::undo_ai_edit,
            commands::get_autostart,
            commands::set_autostart,
            commands::get_dev_mode,
            commands::set_dev_mode,
            commands::get_context_preferences,
            commands::set_context_preferences,
            commands::get_output_policy_config,
            commands::set_output_policy_config,
            commands::get_scratchpad_notes,
            commands::delete_scratchpad_note,
            commands::get_snippets,
            commands::check_for_update,
            commands::install_update,
            commands::get_feature_settings,
            commands::set_feature_settings,
            commands::get_local_model_status,
            commands::download_local_model,
            commands::remove_local_model,
            commands::set_snippets,
            commands::get_vocabulary_suggestions,
            commands::resolve_vocabulary_suggestion,
            commands::get_insights,
            commands::set_insights_backfill_paused,
            commands::set_ai_voice_profile_enabled,
            commands::generate_ai_voice_profile,
            commands::add_insight_correction_to_vocabulary,
            commands::get_sanitizer_enabled,
            commands::set_sanitizer_enabled,
        ])
        .on_window_event(|window, event| {
            // Diagnostic logging of focus / lifecycle / resize events for the
            // main and gadget windows. `Moved` is excluded (it fires per-pixel
            // during a drag and would flood the log). This trail is the key
            // forensic record when the app AppHangs: the last logged event
            // before the gap in app.log shows what the main thread was doing —
            // especially the focus transitions, which correlate with the
            // "crasha quando foco em outra tela" symptom.
            match &event {
                WindowEvent::Focused(focused) => {
                    log::info!("window: '{}' focus -> {}", window.label(), focused);
                }
                WindowEvent::Destroyed => {
                    log::info!("window: '{}' destroyed", window.label());
                    #[cfg(target_os = "windows")]
                    if window.label() == "gadget" {
                        GADGET_HWND.store(0, Ordering::Release);
                    }
                }
                WindowEvent::Resized(size) => {
                    log::info!("window: '{}' resized to {:?}", window.label(), size);
                }
                _ => {}
            }

            // Closing the main window only hides it: the app keeps running in
            // the system tray. "Sair" in the tray menu performs a real exit.
            // The gadget is lifecycle-managed by its state machine; a close
            // request must not destroy the reusable WebView.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let _ = window.hide();
                }
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Sonora application");
}

#[cfg(test)]
mod gadget_placement_tests {
    use super::{
        bottom_center_placement, gadget_geometry, gadget_hit_rect,
        gadget_tracks_foreground_monitor, rectangles_overlap, select_gadget_anchor,
    };
    use crate::models::{GadgetSessionAnchor, GadgetVisualState};

    fn anchor(name: &str, work_x: i32) -> GadgetSessionAnchor {
        GadgetSessionAnchor {
            display_name: Some(name.to_string()),
            work_x,
            work_y: 0,
            work_width: 1920,
            work_height: 1040,
            scale: 1.0,
        }
    }

    #[test]
    fn foreground_monitor_wins_even_when_a_display_is_configured() {
        let selected = select_gadget_anchor(
            Some(anchor("configured", 1)),
            Some(anchor("foreground", 2)),
            Some(anchor("cursor", 3)),
            Some(anchor("primary", 4)),
        )
        .unwrap();
        assert_eq!(selected.display_name.as_deref(), Some("foreground"));
    }

    #[test]
    fn foreground_monitor_wins_when_no_display_is_configured() {
        let selected = select_gadget_anchor(
            None,
            Some(anchor("foreground", 2)),
            Some(anchor("cursor", 3)),
            Some(anchor("primary", 4)),
        )
        .unwrap();
        assert_eq!(selected.display_name.as_deref(), Some("foreground"));
    }

    #[test]
    fn cursor_and_primary_remain_bounded_fallbacks() {
        let cursor = select_gadget_anchor(
            None,
            None,
            Some(anchor("cursor", 3)),
            Some(anchor("primary", 4)),
        )
        .unwrap();
        assert_eq!(cursor.display_name.as_deref(), Some("cursor"));

        let primary = select_gadget_anchor(None, None, None, Some(anchor("primary", 4))).unwrap();
        assert_eq!(primary.display_name.as_deref(), Some("primary"));
    }

    #[test]
    fn centers_recording_on_negative_mixed_dpi_work_area() {
        let anchor = GadgetSessionAnchor {
            display_name: Some("DISPLAY-2".to_string()),
            work_x: -2560,
            work_y: -200,
            work_width: 2560,
            work_height: 1392,
            scale: 1.5,
        };
        let placement =
            bottom_center_placement(&anchor, gadget_geometry(GadgetVisualState::Recording));
        assert_eq!(placement, (-1420, 1093, 279, 72));
    }

    #[test]
    fn uses_work_area_not_full_monitor_height() {
        let anchor = GadgetSessionAnchor {
            display_name: None,
            work_x: 0,
            work_y: 0,
            work_width: 1920,
            work_height: 1040,
            scale: 1.0,
        };
        let placement = bottom_center_placement(&anchor, gadget_geometry(GadgetVisualState::Idle));
        assert_eq!(placement, (924, 970, 72, 52));
    }

    #[test]
    fn every_visual_state_has_a_positive_native_footprint() {
        use GadgetVisualState::*;
        for state in [
            Hidden,
            Idle,
            Hover,
            Appearing,
            Initializing,
            Recording,
            Stopping,
            Processing,
            ProcessingLong,
            Success,
            Error,
        ] {
            let geometry = gadget_geometry(state);
            assert!(geometry.width > 0.0 && geometry.height > 0.0);
        }
    }

    #[test]
    fn every_visible_state_has_a_centered_hit_rect_inside_its_window() {
        use GadgetVisualState::*;
        for state in [
            Idle,
            Hover,
            Appearing,
            Initializing,
            Recording,
            Stopping,
            Processing,
            ProcessingLong,
            Success,
            Error,
        ] {
            let geometry = gadget_geometry(state);
            let rect = gadget_hit_rect(state).expect("visible state must have a hit rect");
            assert!(rect.width > 0.0 && rect.height > 0.0);
            assert!(rect.x >= 0.0 && rect.y >= 0.0);
            assert!(rect.x + rect.width <= geometry.width);
            assert!(rect.y + rect.height <= geometry.height);
            assert!(((rect.x * 2.0 + rect.width) - geometry.width).abs() < f64::EPSILON);
            assert!(((rect.y * 2.0 + rect.height) - geometry.height).abs() < f64::EPSILON);
        }
        assert!(gadget_hit_rect(Hidden).is_none());
    }

    #[test]
    fn z_order_guard_only_treats_real_area_intersection_as_obscured() {
        let gadget = (900, 980, 1_020, 1_040);
        assert!(rectangles_overlap(gadget, (0, 0, 1_920, 1_080)));
        assert!(rectangles_overlap(gadget, (1_000, 1_000, 1_200, 1_100)));
        assert!(!rectangles_overlap(gadget, (0, 0, 900, 1_080)));
        assert!(!rectangles_overlap(gadget, (0, 1_040, 1_920, 1_080)));
        assert!(!rectangles_overlap(gadget, (0, 0, 0, 0)));
    }

    #[test]
    fn every_visible_gadget_state_follows_the_foreground_monitor() {
        use GadgetVisualState::*;
        assert!(!gadget_tracks_foreground_monitor(Hidden));
        for state in [
            Idle,
            Hover,
            Appearing,
            Initializing,
            Recording,
            Stopping,
            Processing,
            ProcessingLong,
            Success,
            Error,
        ] {
            assert!(gadget_tracks_foreground_monitor(state), "{state:?}");
        }
    }
}

#[cfg(test)]
mod log_rotation_tests {
    use super::*;
    #[test]
    fn open_log_can_rotate_on_windows_without_losing_previous_bytes() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("sonora-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.log");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(8 * 1024 * 1024)
            .unwrap();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        let mut writer = TeeWriter {
            file,
            path: path.clone(),
        };
        writer.write_all(b"synthetic-log\n").unwrap();
        writer.flush().unwrap();
        drop(writer);
        assert_eq!(
            std::fs::metadata(dir.join("app.1.log")).unwrap().len(),
            8 * 1024 * 1024
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"synthetic-log\n");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
