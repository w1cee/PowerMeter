use crate::win;
use std::process::Command;
use windows::Win32::System::Com::Urlmon::URLDownloadToFileW;
use windows::core::HSTRING;

const URL: &str = "https://github.com/namazso/PawnIO.Setup/releases/latest/download/PawnIO_setup.exe";
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO";

pub fn installed_version() -> Option<(u32, u32)> {
    let text = win::reg_string(win::hklm(), UNINSTALL_KEY, "DisplayVersion")?;
    let mut parts = text.split('.').map(|p| p.trim().parse::<u32>().unwrap_or(0));
    Some((parts.next()?, parts.next().unwrap_or(0)))
}

pub fn is_ready() -> bool {
    installed_version().is_some_and(|v| v >= (2, 0))
}

/// Downloads the official installer and runs it. Blocking: call from a worker thread.
pub fn install(progress: impl Fn(&str)) -> Result<(), String> {
    let path = std::env::temp_dir().join("PawnIO_setup.exe");

    progress("Downloading PawnIO…");
    unsafe {
        URLDownloadToFileW(None, &HSTRING::from(URL), &HSTRING::from(path.as_os_str()), 0, None)
            .map_err(|e| format!("Download failed: {}", e.message()))?;
    }

    progress("Installing PawnIO…");
    let status = Command::new(&path).arg("-install").status().map_err(|e| format!("Couldn't start the installer: {e}"))?;
    let _ = std::fs::remove_file(&path);

    if is_ready() {
        Ok(())
    } else {
        Err(format!("The installer exited with code {}, but PawnIO is still missing", status.code().unwrap_or(-1)))
    }
}
