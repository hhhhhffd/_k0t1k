//! The few system calls onboarding and updates need: waiting for a process to exit (Windows and Linux)
//! and creating `.lnk` shortcuts (Windows). Elsewhere they fail with an explicit error.

use std::path::Path;
use std::time::Duration;

use crate::error::{AppError, CommandResult};

/// Waits until the process `pid` has exited. Returns `false` when it is still running after `timeout`.
#[cfg(windows)]
pub fn wait_for_exit(pid: u32, timeout: Duration) -> CommandResult<bool> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

    // SAFETY: the handle is valid until the CloseHandle below and is not used after it.
    unsafe {
        let handle = match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(handle) => handle,
            // No process with this id: it has already exited.
            Err(error) if error.code() == ERROR_INVALID_PARAMETER.to_hresult() => return Ok(true),
            Err(error) => return Err(AppError::new(format!("cannot open process {pid}: {error}"))),
        };
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let outcome = WaitForSingleObject(handle, millis);
        CloseHandle(handle).map_err(|error| AppError::new(format!("cannot close the handle of process {pid}: {error}")))?;
        Ok(outcome == WAIT_OBJECT_0)
    }
}

#[cfg(target_os = "linux")]
pub fn wait_for_exit(pid: u32, timeout: Duration) -> CommandResult<bool> {
    let stat = std::path::PathBuf::from(format!("/proc/{pid}/stat"));
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match std::fs::read_to_string(&stat) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(AppError::new(format!("cannot read {}: {error}", stat.display()))),
            // A zombie has exited; only its parent has not collected the exit status yet.
            Ok(text) if text.rsplit_once(") ").is_some_and(|(_, rest)| rest.starts_with('Z')) => return Ok(true),
            Ok(_) => {}
        }
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn wait_for_exit(_pid: u32, _timeout: Duration) -> CommandResult<bool> {
    Err(AppError::new("waiting for a process is only implemented on Windows and Linux"))
}

/// Writes the shortcut `lnk` that starts `target`, with the target's folder as working directory.
#[cfg(windows)]
pub fn create_shortcut(lnk: &Path, target: &Path) -> CommandResult<()> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface, w};

    let working_dir = target.parent().ok_or_else(|| AppError::new(format!("{} has no folder", target.display())))?;
    let failed = |error: windows::core::Error| AppError::new(format!("cannot create the shortcut {}: {error}", lnk.display()));
    // SAFETY: COM is initialised on this thread before use and released after the last COM object is dropped.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(failed)?;
        let result = (|| {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(target.as_os_str()))?;
            link.SetWorkingDirectory(&HSTRING::from(working_dir.as_os_str()))?;
            link.SetDescription(w!("_k0t1k"))?;
            link.cast::<IPersistFile>()?.Save(&HSTRING::from(lnk.as_os_str()), true)
        })();
        CoUninitialize();
        result.map_err(failed)
    }
}

#[cfg(not(windows))]
pub fn create_shortcut(_lnk: &Path, _target: &Path) -> CommandResult<()> {
    Err(AppError::new("shortcuts are only implemented on Windows"))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn waits_for_an_exited_process_and_gives_up_on_a_running_one() {
        let mut quick = std::process::Command::new("sleep").arg("0.2").spawn().unwrap();
        // Not reaped until `wait` below, so it ends as a zombie, which counts as exited.
        assert!(wait_for_exit(quick.id(), Duration::from_secs(5)).unwrap());
        quick.wait().unwrap();
        assert!(wait_for_exit(quick.id(), Duration::from_secs(1)).unwrap());

        let mut slow = std::process::Command::new("sleep").arg("5").spawn().unwrap();
        assert!(!wait_for_exit(slow.id(), Duration::from_millis(300)).unwrap());
        slow.kill().unwrap();
        slow.wait().unwrap();
    }
}
