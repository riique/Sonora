//! Windows integrity-level helpers.
//!
//! User Interface Privilege Isolation (UIPI) silently drops synthetic input
//! (`SendInput`) sent from a process to a window owned by a process with a
//! higher integrity level. When the user dictates into an app running "as
//! administrator", our simulated `Ctrl+V` never arrives. The only reliable
//! fix is for Sonora itself to run elevated, so this module detects the
//! mismatch and can relaunch the app with the `runas` verb.

/// Argument passed to the elevated child so it waits for the parent to
/// release the single-instance mutex instead of exiting immediately.
pub const ELEVATED_RELAUNCH_ARG: &str = "--elevated-relaunch";

#[cfg(windows)]
mod imp {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{
            GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenElevation,
            TokenIntegrityLevel, TOKEN_ELEVATION, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
        },
        System::Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn process_token(process: HANDLE) -> Option<Handle> {
        let mut token = HANDLE::default();
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.ok()?;
        Some(Handle(token))
    }

    fn integrity_level(token: &Handle) -> Option<u32> {
        unsafe {
            let mut size = 0_u32;
            let _ = GetTokenInformation(token.0, TokenIntegrityLevel, None, 0, &mut size);
            if size == 0 {
                return None;
            }
            let mut buffer = vec![0_u8; size as usize];
            GetTokenInformation(
                token.0,
                TokenIntegrityLevel,
                Some(buffer.as_mut_ptr().cast()),
                size,
                &mut size,
            )
            .ok()?;
            let label = &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL);
            let sid = label.Label.Sid;
            let count = *GetSidSubAuthorityCount(sid);
            if count == 0 {
                return None;
            }
            Some(*GetSidSubAuthority(sid, u32::from(count) - 1))
        }
    }

    pub fn current_process_elevated() -> bool {
        let Some(token) = process_token(unsafe { GetCurrentProcess() }) else {
            return false;
        };
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
        unsafe {
            GetTokenInformation(
                token.0,
                TokenElevation,
                Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
                size,
                &mut size,
            )
        }
        .is_ok()
            && elevation.TokenIsElevated != 0
    }

    /// `Some(true)` when `process_id` runs at a higher integrity level than
    /// Sonora (UIPI will block our input), `None` when it cannot be determined.
    pub fn process_outranks_us(process_id: u32) -> Option<bool> {
        let ours = integrity_level(&process_token(unsafe { GetCurrentProcess() })?)?;
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
            .ok()
            .map(Handle);
        let theirs = match process
            .as_ref()
            .and_then(|process| process_token(process.0))
        {
            Some(token) => integrity_level(&token)?,
            // A same-user, same-integrity process is always queryable; being
            // denied means the target is more privileged than we are.
            None => return Some(!current_process_elevated()),
        };
        Some(theirs > ours)
    }

    pub fn relaunch_elevated() -> Result<(), String> {
        use windows::{
            core::{w, HSTRING, PCWSTR},
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut args = vec![super::ELEVATED_RELAUNCH_ARG.to_string()];
        if std::env::args().any(|arg| arg == "--autostart") {
            args.push("--autostart".into());
        }
        let exe = HSTRING::from(exe.as_os_str());
        let params = HSTRING::from(args.join(" "));
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("runas"),
                &exe,
                &params,
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecute reports success with a value greater than 32.
        if result.0 as isize > 32 {
            Ok(())
        } else {
            Err("Elevação cancelada ou negada pelo Windows.".into())
        }
    }
}

#[cfg(windows)]
pub use imp::{current_process_elevated, process_outranks_us, relaunch_elevated};

#[cfg(not(windows))]
pub fn current_process_elevated() -> bool {
    false
}

#[cfg(not(windows))]
pub fn process_outranks_us(_process_id: u32) -> Option<bool> {
    Some(false)
}

#[cfg(not(windows))]
pub fn relaunch_elevated() -> Result<(), String> {
    Err("Disponível apenas no Windows.".into())
}

/// Starts an elevated copy of Sonora and terminates this one. Returns only
/// when elevation failed (e.g. the user dismissed the UAC prompt).
pub fn restart_elevated() -> Result<(), String> {
    relaunch_elevated()?;
    log::info!("elevation: elevated instance launched, exiting");
    std::process::exit(0);
}
