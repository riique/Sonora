//! Privacy-aware context captured at recording start.

use serde::{Deserialize, Serialize};
use std::hash::Hasher;
use std::sync::OnceLock;
use std::time::Duration;

use crate::pipeline_run::epoch_ms;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSourceKind {
    #[default]
    Application,
    WindowTitle,
    Domain,
    Selection,
    CaretContext,
    Clipboard,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextPrivacy {
    #[default]
    MetadataOnly,
    EphemeralLocal,
    CloudAllowed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSourcePreference {
    pub source: ContextSourceKind,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub privacy: ContextPrivacy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPreferences {
    #[serde(default)]
    pub sources: Vec<ContextSourcePreference>,
    #[serde(default)]
    pub persist_raw_context: bool,
    #[serde(default)]
    pub allow_context_to_cloud: bool,
    #[serde(default = "default_context_chars")]
    pub max_context_chars: usize,
}

fn default_context_chars() -> usize {
    800
}

impl Default for ContextPreferences {
    fn default() -> Self {
        Self {
            sources: vec![
                ContextSourcePreference {
                    source: ContextSourceKind::Application,
                    enabled: true,
                    privacy: ContextPrivacy::MetadataOnly,
                },
                ContextSourcePreference {
                    source: ContextSourceKind::WindowTitle,
                    enabled: true,
                    privacy: ContextPrivacy::MetadataOnly,
                },
                ContextSourcePreference {
                    source: ContextSourceKind::Domain,
                    enabled: true,
                    privacy: ContextPrivacy::MetadataOnly,
                },
                ContextSourcePreference {
                    source: ContextSourceKind::Selection,
                    enabled: false,
                    privacy: ContextPrivacy::EphemeralLocal,
                },
                ContextSourcePreference {
                    source: ContextSourceKind::CaretContext,
                    enabled: false,
                    privacy: ContextPrivacy::EphemeralLocal,
                },
                ContextSourcePreference {
                    source: ContextSourceKind::Clipboard,
                    enabled: false,
                    privacy: ContextPrivacy::EphemeralLocal,
                },
            ],
            persist_raw_context: false,
            allow_context_to_cloud: false,
            max_context_chars: default_context_chars(),
        }
    }
}

impl ContextPreferences {
    pub fn source(&self, source: ContextSourceKind) -> ContextSourcePreference {
        self.sources
            .iter()
            .find(|item| item.source == source)
            .cloned()
            .unwrap_or(ContextSourcePreference {
                source,
                enabled: false,
                privacy: ContextPrivacy::MetadataOnly,
            })
    }

    pub fn enabled(&self, source: ContextSourceKind) -> bool {
        self.source(source).enabled
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextFailure {
    pub source: ContextSourceKind,
    pub code: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserContext {
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub selection: Option<String>,
    #[serde(default)]
    pub nearby_text: Option<String>,
    #[serde(default)]
    pub captured_at_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSnapshot {
    #[serde(default)]
    pub captured_at_ms: u64,
    #[serde(default)]
    pub foreground_hwnd: Option<isize>,
    #[serde(default)]
    pub process_id: Option<u32>,
    #[serde(default)]
    pub process: Option<String>,
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub window_title: Option<String>,
    #[serde(default)]
    pub window_class: Option<String>,
    #[serde(default)]
    pub monitor: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub selected_text: Option<String>,
    #[serde(default)]
    pub caret_text: Option<String>,
    #[serde(default)]
    pub clipboard_text: Option<String>,
    #[serde(default)]
    pub browser: Option<BrowserContext>,
    #[serde(default)]
    pub failures: Vec<ContextFailure>,
    #[serde(default)]
    pub raw_context_persisted: bool,
    #[serde(default)]
    pub cloud_context_allowed: bool,
}

/// Minimal, privacy-preserving identity of the window that should receive the
/// final paste. It is captured when recording stops, independently from the
/// richer context snapshot collected when recording starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForegroundTarget {
    #[serde(default)]
    pub focus_id: Option<u64>,
    #[serde(default)]
    pub hwnd: Option<isize>,
    #[serde(default)]
    pub process_id: Option<u32>,
}

impl ForegroundTarget {
    pub fn from_snapshot(snapshot: &ContextSnapshot) -> Self {
        Self {
            hwnd: snapshot.foreground_hwnd,
            focus_id: None,
            process_id: snapshot.process_id,
        }
    }
}

fn delivery_targets_match(expected: ForegroundTarget, current: ForegroundTarget) -> bool {
    if expected.focus_id.is_none() || expected.focus_id != current.focus_id {
        return false;
    }
    matches!(
        (
            expected.hwnd,
            expected.process_id,
            current.hwnd,
            current.process_id,
        ),
        (Some(expected_hwnd), Some(expected_pid), Some(current_hwnd), Some(current_pid))
            if expected_hwnd == current_hwnd && expected_pid == current_pid
    )
}

/// Captures the foreground window, process and opaque focused-control identity. No title,
/// selection, clipboard, browser or document content is read.
pub fn capture_foreground_target() -> ForegroundTarget {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };

        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return ForegroundTarget::default();
            }
            let mut process_id = 0_u32;
            GetWindowThreadProcessId(hwnd, Some(&mut process_id));
            let focus_id = focused_control_identity();
            // UIA may cross process boundaries. Do not combine one window's
            // identity with a field captured after the user changed windows.
            if GetForegroundWindow() != hwnd {
                return ForegroundTarget::default();
            }
            ForegroundTarget {
                hwnd: Some(hwnd.0 as isize),
                focus_id,
                process_id: (process_id != 0).then_some(process_id),
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        ForegroundTarget::default()
    }
}

/// Verifies whether the foreground target matches an expected target.
/// Used during undo operations (`undo_ai_edit`) to ensure the user has not switched
/// focus away before restoring previously replaced selection text.
pub fn foreground_delivery_target_matches(expected: ForegroundTarget) -> bool {
    #[cfg(target_os = "windows")]
    {
        delivery_targets_match(expected, capture_foreground_target())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = expected;
        true
    }
}

impl ContextSnapshot {
    pub fn persisted_metadata(&self) -> Self {
        let mut safe = self.clone();
        if !self.raw_context_persisted {
            safe.selected_text = None;
            safe.caret_text = None;
            safe.clipboard_text = None;
            if let Some(browser) = safe.browser.as_mut() {
                browser.selection = None;
                browser.nearby_text = None;
                browser.url = browser
                    .domain
                    .as_ref()
                    .map(|domain| format!("https://{domain}"));
            }
        }
        safe
    }
}

pub fn capture(preferences: &ContextPreferences) -> ContextSnapshot {
    let mut snapshot = capture_platform_metadata(preferences);
    snapshot.captured_at_ms = epoch_ms();
    snapshot.raw_context_persisted = preferences.persist_raw_context;

    if preferences.enabled(ContextSourceKind::Clipboard) {
        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
            Ok(text) => match sanitize_context_text(&text, preferences.max_context_chars) {
                Some(text) => snapshot.clipboard_text = Some(text),
                None => snapshot.failures.push(ContextFailure {
                    source: ContextSourceKind::Clipboard,
                    code: "sensitive_content_filtered".into(),
                    message: None,
                }),
            },
            Err(error) => snapshot.failures.push(ContextFailure {
                source: ContextSourceKind::Clipboard,
                code: "clipboard_unavailable".into(),
                message: Some(error.to_string()),
            }),
        }
    }

    if !preferences.enabled(ContextSourceKind::Application) {
        snapshot.process = None;
        snapshot.executable = None;
        snapshot.window_class = None;
        snapshot.monitor = None;
    }

    snapshot
}

fn sanitize_context_text(value: &str, max_chars: usize) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || looks_sensitive(trimmed) {
        return None;
    }
    Some(trimmed.chars().take(max_chars.max(1)).collect())
}

fn looks_sensitive(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if [
        "password=",
        "password:",
        "passwd=",
        "authorization:",
        "api_key=",
        "api-key:",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return true;
    }
    value.split_whitespace().any(|token| {
        let token = token.trim_matches(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '-' | '_' | '.')
        });
        let prefixed_secret = ["sk-", "gsk_", "ghp_", "AIza", "xoxb-"]
            .iter()
            .any(|prefix| token.starts_with(prefix));
        let jwt = token.len() > 40 && token.matches('.').count() == 2;
        let classes = [
            token
                .chars()
                .any(|character| character.is_ascii_lowercase()),
            token
                .chars()
                .any(|character| character.is_ascii_uppercase()),
            token.chars().any(|character| character.is_ascii_digit()),
            token
                .chars()
                .any(|character| matches!(character, '-' | '_' | '+' | '/' | '=')),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        prefixed_secret || jwt || (token.len() >= 40 && classes >= 3)
    })
}

#[cfg(not(target_os = "windows"))]
fn capture_platform_metadata(_preferences: &ContextPreferences) -> ContextSnapshot {
    ContextSnapshot::default()
}

#[cfg(target_os = "windows")]
fn capture_platform_metadata(preferences: &ContextPreferences) -> ContextSnapshot {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    };

    let mut snapshot = ContextSnapshot::default();
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            snapshot.failures.push(ContextFailure {
                source: ContextSourceKind::Application,
                code: "foreground_window_unavailable".into(),
                message: None,
            });
            return snapshot;
        }
        snapshot.foreground_hwnd = Some(hwnd.0 as isize);

        let mut process_id = 0_u32;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        snapshot.process_id = (process_id != 0).then_some(process_id);

        if preferences.enabled(ContextSourceKind::WindowTitle) {
            let mut buffer = vec![0_u16; 1024];
            let len = GetWindowTextW(hwnd, &mut buffer);
            if len > 0 {
                snapshot.window_title =
                    sanitize_context_text(&String::from_utf16_lossy(&buffer[..len as usize]), 300);
            }
        }

        let mut class_buffer = vec![0_u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_buffer);
        if class_len > 0 {
            snapshot.window_class = Some(String::from_utf16_lossy(
                &class_buffer[..class_len as usize],
            ));
        }

        if process_id != 0 {
            if let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) {
                let mut path_buffer = vec![0_u16; 32_768];
                let mut size = path_buffer.len() as u32;
                if QueryFullProcessImageNameW(
                    process,
                    PROCESS_NAME_WIN32,
                    PWSTR(path_buffer.as_mut_ptr()),
                    &mut size,
                )
                .is_ok()
                {
                    let executable = String::from_utf16_lossy(&path_buffer[..size as usize]);
                    snapshot.process = std::path::Path::new(&executable)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned());
                    snapshot.executable = Some(executable);
                }
                let _ = CloseHandle(process);
            }
        }

        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if !monitor.0.is_null() {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
                let length = info
                    .szDevice
                    .iter()
                    .position(|value| *value == 0)
                    .unwrap_or(info.szDevice.len());
                snapshot.monitor = Some(String::from_utf16_lossy(&info.szDevice[..length]));
            }
        }

        if preferences.enabled(ContextSourceKind::Selection)
            || preferences.enabled(ContextSourceKind::CaretContext)
        {
            match capture_uia_text(preferences.max_context_chars) {
                Ok((is_password, selection, caret)) => {
                    if is_password {
                        snapshot.failures.push(ContextFailure {
                            source: ContextSourceKind::Selection,
                            code: "password_field_filtered".into(),
                            message: None,
                        });
                    } else {
                        if preferences.enabled(ContextSourceKind::Selection) {
                            snapshot.selected_text = selection;
                        }
                        if preferences.enabled(ContextSourceKind::CaretContext) {
                            snapshot.caret_text = caret;
                        }
                    }
                }
                Err(message) => snapshot.failures.push(ContextFailure {
                    source: ContextSourceKind::CaretContext,
                    code: "uia_unavailable".into(),
                    message: Some(message),
                }),
            }
        }
    }
    snapshot
}

#[cfg(target_os = "windows")]
unsafe fn capture_uia_text(
    max_chars: usize,
) -> Result<(bool, Option<String>, Option<String>), String> {
    use windows::core::Interface;
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationTextPattern, IUIAutomationTextPattern2,
        TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start, TextUnit_Character,
        UIA_TextPattern2Id, UIA_TextPatternId,
    };

    let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
    let result = (|| {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| error.to_string())?;
        let focused = automation
            .GetFocusedElement()
            .map_err(|error| error.to_string())?;
        let is_password = focused
            .CurrentIsPassword()
            .map(|value| value.as_bool())
            .unwrap_or(true);
        if is_password {
            return Ok((true, None, None));
        }

        let (pattern, caret_range) =
            match focused.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id) {
                Ok(pattern2) => {
                    let mut active = BOOL::default();
                    let caret = pattern2.GetCaretRange(&mut active).ok();
                    (
                        pattern2
                            .cast::<IUIAutomationTextPattern>()
                            .map_err(|error| error.to_string())?,
                        caret,
                    )
                }
                Err(_) => (
                    focused
                        .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                        .map_err(|error| error.to_string())?,
                    None,
                ),
            };

        let selection_ranges = pattern.GetSelection().map_err(|error| error.to_string())?;
        let first_selection = (selection_ranges.Length().unwrap_or_default() > 0)
            .then(|| selection_ranges.GetElement(0).ok())
            .flatten();
        let selection = first_selection
            .as_ref()
            .and_then(|range| range.GetText(max_chars as i32).ok())
            .and_then(|text| sanitize_context_text(&text.to_string(), max_chars));

        let caret_source = caret_range.or(first_selection);
        let caret = caret_source.and_then(|range| {
            let expanded = range.Clone().ok()?;
            let side = (max_chars.min(800) / 2) as i32;
            let _ = expanded.MoveEndpointByUnit(
                TextPatternRangeEndpoint_Start,
                TextUnit_Character,
                -side,
            );
            let _ =
                expanded.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, side);
            let text = expanded.GetText(max_chars as i32).ok()?.to_string();
            sanitize_context_text(&text, max_chars)
        });
        Ok((false, selection, caret))
    })();
    if initialized {
        CoUninitialize();
    }
    result
}

/// Keep the UIA client and COM apartment alive between stop and delivery.
/// Recreating a client on the UI/shortcut thread and disposing its apartment
/// immediately can invalidate Chromium's accessibility provider identities.
/// All focus queries use one bounded, windowless MTA worker; a stalled or
/// unavailable provider still fails closed. No field text or names are read.
#[cfg(windows)]
fn focused_control_identity() -> Option<u64> {
    use std::sync::mpsc::{self, SyncSender};
    use std::time::Instant;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

    struct FocusRequest {
        expires_at: Instant,
        reply: SyncSender<Option<u64>>,
    }
    static READER: OnceLock<Option<SyncSender<FocusRequest>>> = OnceLock::new();
    let reader = READER
        .get_or_init(|| {
            let (send, receive) = mpsc::sync_channel::<FocusRequest>(1);
            std::thread::Builder::new()
                .name("sonora-focus-reader".into())
                .spawn(move || unsafe {
                    if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                        return;
                    }
                    {
                        let automation: windows::core::Result<IUIAutomation> =
                            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER);
                        if let Ok(automation) = automation {
                            while let Ok(request) = receive.recv() {
                                let identity = (Instant::now() < request.expires_at)
                                    .then(|| hash_focused_control(&automation))
                                    .flatten();
                                let _ = request.reply.try_send(identity);
                            }
                        }
                    }
                    CoUninitialize();
                })
                .ok()?;
            Some(send)
        })
        .as_ref()?;
    let timeout = Duration::from_millis(700);
    let (reply, receive) = mpsc::sync_channel(1);
    reader
        .try_send(FocusRequest {
            expires_at: Instant::now() + timeout,
            reply,
        })
        .ok()?;
    receive.recv_timeout(timeout).ok().flatten()
}

#[cfg(windows)]
unsafe fn hash_focused_control(
    automation: &windows::Win32::UI::Accessibility::IUIAutomation,
) -> Option<u64> {
    use windows::Win32::System::Ole::*;
    let element = automation.GetFocusedElement().ok()?;
    if element.CurrentIsPassword().ok()?.as_bool() {
        return None;
    }
    let array = element.GetRuntimeId().ok()?;
    if array.is_null() {
        return None;
    }
    let identity = (|| {
        let lower = SafeArrayGetLBound(array, 1).ok()?;
        let upper = SafeArrayGetUBound(array, 1).ok()?;
        let len = usize::try_from(upper.checked_sub(lower)?.checked_add(1)?).ok()?;
        if len == 0 || len > 128 {
            return None;
        }
        let mut data = std::ptr::null_mut();
        SafeArrayAccessData(array, &mut data).ok()?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for part in std::slice::from_raw_parts(data.cast::<i32>(), len) {
            hash.write_i32(*part);
        }
        let _ = SafeArrayUnaccessData(array);
        Some(hash.finish())
    })();
    let _ = SafeArrayDestroy(array);
    identity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_and_high_entropy_tokens_are_filtered() {
        assert!(sanitize_context_text("password=secret", 200).is_none());
        assert!(sanitize_context_text("sk-abcdefghijklmnopqrstuvwxyz123456", 200).is_none());
        assert_eq!(
            sanitize_context_text("OpenRouter no Codex", 200).as_deref(),
            Some("OpenRouter no Codex")
        );
    }

    #[test]
    fn persisted_metadata_drops_raw_context_by_default() {
        let snapshot = ContextSnapshot {
            selected_text: Some("seleção".into()),
            clipboard_text: Some("clipboard".into()),
            ..Default::default()
        };
        let persisted = snapshot.persisted_metadata();
        assert!(persisted.selected_text.is_none());
        assert!(persisted.clipboard_text.is_none());
    }

    #[test]
    fn delivery_target_survives_monitor_moves_but_rejects_other_windows() {
        let expected = ForegroundTarget {
            hwnd: Some(101),
            process_id: Some(7),
            focus_id: Some(1),
        };
        assert!(delivery_targets_match(expected, expected));
        assert!(!delivery_targets_match(
            expected,
            ForegroundTarget {
                hwnd: Some(202),
                process_id: Some(7),
                focus_id: Some(1),
            }
        ));
        assert!(!delivery_targets_match(
            expected,
            ForegroundTarget {
                hwnd: Some(101),
                process_id: Some(8),
                focus_id: Some(1),
            }
        ));
        assert!(!delivery_targets_match(
            expected,
            ForegroundTarget::default()
        ));
        assert!(!delivery_targets_match(
            expected,
            ForegroundTarget {
                focus_id: Some(2),
                ..expected
            }
        ));
        assert!(!delivery_targets_match(
            expected,
            ForegroundTarget {
                focus_id: None,
                ..expected
            }
        ));
        assert!(!delivery_targets_match(
            ForegroundTarget {
                focus_id: None,
                ..expected
            },
            ForegroundTarget {
                focus_id: None,
                ..expected
            }
        ));
    }

    #[test]
    fn switching_apps_while_recording_keeps_start_context_but_delivers_to_stop_field() {
        use crate::pipeline_run::{PipelineRun, RecordingSession};
        let chrome = ContextSnapshot {
            foreground_hwnd: Some(101),
            process_id: Some(7),
            ..Default::default()
        };
        let codex = ForegroundTarget {
            hwnd: Some(202),
            process_id: Some(8),
            focus_id: Some(99),
        };
        let mut session = RecordingSession {
            context: chrome.clone(),
            delivery_target: ForegroundTarget::from_snapshot(&chrome),
            ..Default::default()
        };
        session.delivery_target = codex;
        let run = PipelineRun::new(
            "switch-apps",
            &session,
            crate::pipeline_contract::TranscriptionMode::UltraFast,
        );
        assert_eq!(run.context.foreground_hwnd, chrome.foreground_hwnd);
        let delivered_to = ForegroundTarget {
            hwnd: run.delivery.target_hwnd,
            process_id: run.delivery.target_process_id,
            focus_id: run.delivery.target_focus_id,
        };
        assert!(delivery_targets_match(delivered_to, codex));
        assert!(!delivery_targets_match(
            delivered_to,
            ForegroundTarget::from_snapshot(&chrome)
        ));
        assert!(!delivery_targets_match(
            delivered_to,
            ForegroundTarget {
                focus_id: Some(100),
                ..codex
            }
        ));
    }

    /// Run explicitly with a non-password field focused and leave focus there.
    /// This reads opaque identities only: no microphone, clipboard or provider.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires an interactive Windows desktop with a stable focused field"]
    fn windows_focus_identity_survives_repeated_queries_from_different_threads() {
        let initial = capture_foreground_target();
        assert!(
            initial.focus_id.is_some(),
            "focus a normal editable field before running"
        );
        for _ in 0..8 {
            std::thread::sleep(Duration::from_millis(250));
            let current = std::thread::spawn(capture_foreground_target)
                .join()
                .unwrap();
            assert!(
                delivery_targets_match(initial, current),
                "keep the same field focused during this check"
            );
        }
    }
}
