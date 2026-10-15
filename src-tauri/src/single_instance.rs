#[cfg(windows)]
pub struct Instance(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(windows)]
pub fn acquire() -> Result<Option<Instance>, String> {
    use windows::{
        core::w,
        Win32::{
            Foundation::{GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS},
            System::Threading::CreateMutexW,
            UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE},
        },
    };
    // An elevated relaunch starts while the parent still holds the mutex;
    // give the parent a moment to exit before treating it as a duplicate.
    let relaunching = std::env::args().any(|arg| arg == crate::elevation::ELEVATED_RELAUNCH_ARG);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    unsafe {
        loop {
            match CreateMutexW(
                None,
                false,
                w!("Local\\HaumeaVoice.Tauri.SingleInstance.v1"),
            ) {
                Ok(mutex) if GetLastError() != ERROR_ALREADY_EXISTS => {
                    return Ok(Some(Instance(mutex)));
                }
                Ok(mutex) => drop(Instance(mutex)),
                // A mutex created by an elevated instance cannot be opened by
                // a normal one: that elevated instance is already running.
                Err(error) if error.code() == ERROR_ACCESS_DENIED.to_hresult() => {}
                Err(error) => return Err(error.to_string()),
            }
            if !relaunching || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if let Ok(window) = FindWindowW(None, w!("Sonora")) {
            let _ = ShowWindow(window, SW_RESTORE);
            let _ = SetForegroundWindow(window);
        }
        Ok(None)
    }
}
#[cfg(not(windows))]
pub fn acquire() -> Result<Option<()>, String> {
    Ok(Some(()))
}
