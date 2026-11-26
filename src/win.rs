use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWINDOWATTRIBUTE, DwmSetWindowAttribute};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{HKEY, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    ICON_BIG, ICON_SMALL, IDYES, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION,
    MB_OK, MB_YESNO, MESSAGEBOX_STYLE, MessageBoxW, SendMessageW, WM_SETICON,
};
use windows::core::{HSTRING, PCWSTR};

pub type Window = HWND;

pub fn window_from(frame: &eframe::Frame) -> Option<Window> {
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as _)),
        _ => None,
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn message(hwnd: Option<HWND>, text: &str, title: &str, style: MESSAGEBOX_STYLE) -> i32 {
    unsafe { MessageBoxW(hwnd, &HSTRING::from(text), &HSTRING::from(title), style).0 }
}

pub fn info(hwnd: Option<HWND>, text: &str) {
    message(hwnd, text, "PowerMeter", MB_OK | MB_ICONINFORMATION);
}

pub fn error(hwnd: Option<HWND>, title: &str, text: &str) {
    message(hwnd, text, title, MB_OK | MB_ICONERROR);
}

pub fn confirm(hwnd: Option<HWND>, text: &str) -> bool {
    message(hwnd, text, "PowerMeter", MB_YESNO | MB_ICONQUESTION) == IDYES.0
}

/// Returns false if another instance already holds the mutex.
pub fn single_instance() -> bool {
    unsafe {
        // never closed on purpose: the mutex lives as long as the process
        let handle = CreateMutexW(None, true, &HSTRING::from("Local\\PowerMeter.SingleInstance"));
        handle.is_ok() && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

pub fn style_window(hwnd: HWND) {
    unsafe {
        let on: i32 = 1;
        let size = size_of::<i32>() as u32;
        let ptr = &on as *const i32 as *const _;
        if DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), ptr, size).is_err() {
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(19), ptr, size);
        }

        // winit only knows the 64px icon; give Windows the real multi-size one from resources
        if let Ok(module) = GetModuleHandleW(None) {
            for (kind, size) in [(ICON_BIG, 32), (ICON_SMALL, 16)] {
                if let Ok(icon) = LoadImageW(Some(module.into()), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_DEFAULTCOLOR) {
                    SendMessageW(hwnd, WM_SETICON, Some(WPARAM(kind as usize)), Some(LPARAM(icon.0 as isize)));
                }
            }
        }
    }
}

pub fn reg_string(root: HKEY, path: &str, name: &str) -> Option<String> {
    let mut buf = [0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(path),
            &HSTRING::from(name),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }
    Some(from_wide(&buf))
}

pub fn reg_dword(root: HKEY, path: &str, name: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut size = 4u32;
    unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(path),
            &HSTRING::from(name),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }
    Some(value)
}

pub fn hklm() -> HKEY {
    HKEY_LOCAL_MACHINE
}

pub struct OwnedHandle(pub HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

unsafe impl Send for OwnedHandle {}
