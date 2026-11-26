//! Tray icon on its own thread with a plain Win32 message loop, so it keeps
//! working while the egui window is hidden.

use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const WM_TRAY: u32 = WM_APP + 1;
const ID_OPEN: usize = 1;
const ID_QUIT: usize = 2;
const TIMER: usize = 1;

static TOOLTIP: Mutex<String> = Mutex::new(String::new());
static MAIN: AtomicIsize = AtomicIsize::new(0);
static TRAY: AtomicIsize = AtomicIsize::new(0);

pub fn set_main(hwnd: HWND) {
    MAIN.store(hwnd.0 as isize, Ordering::Relaxed);
}

pub fn set_tooltip(text: String) {
    *TOOLTIP.lock().unwrap() = text;
}

fn main_window() -> Option<HWND> {
    match MAIN.load(Ordering::Relaxed) {
        0 => None,
        h => Some(HWND(h as _)),
    }
}

pub fn show_main() {
    if let Some(hwnd) = main_window() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

pub fn hide_main() {
    if let Some(hwnd) = main_window() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

pub fn remove() {
    let tray = TRAY.load(Ordering::Relaxed);
    if tray != 0 {
        unsafe {
            let _ = SendMessageW(HWND(tray as _), WM_CLOSE, None, None);
        }
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        ..Default::default()
    };
    unsafe {
        if let Ok(module) = GetModuleHandleW(None) {
            if let Ok(icon) = LoadImageW(Some(module.into()), PCWSTR(1 as _), IMAGE_ICON, 16, 16, LR_DEFAULTCOLOR) {
                data.hIcon = HICON(icon.0);
            }
        }
    }
    let tip = TOOLTIP.lock().unwrap();
    let tip = if tip.is_empty() { "PowerMeter" } else { tip.as_str() };
    for (dst, src) in data.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
        *dst = src;
    }
    data
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TRAY => {
                match lparam.0 as u32 {
                    WM_LBUTTONDBLCLK | WM_LBUTTONUP => show_main(),
                    WM_RBUTTONUP => menu(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_TIMER => {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &icon_data(hwnd));
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd));
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, w!("Open PowerMeter"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        match cmd {
            ID_OPEN => show_main(),
            ID_QUIT => {
                if let Some(main) = main_window() {
                    let _ = ShowWindow(main, SW_SHOW);
                    let _ = PostMessageW(Some(main), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            _ => {}
        }
    }
}

pub fn spawn() {
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(|| unsafe {
            let Ok(module) = GetModuleHandleW(None) else { return };
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: module.into(),
                lpszClassName: w!("PowerMeterTray"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let Ok(hwnd) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("PowerMeterTray"),
                w!("PowerMeterTray"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(module.into()),
                None,
            ) else {
                return;
            };
            TRAY.store(hwnd.0 as isize, Ordering::Relaxed);
            let _ = Shell_NotifyIconW(NIM_ADD, &icon_data(hwnd));
            SetTimer(Some(hwnd), TIMER, 2000, None);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            TRAY.store(0, Ordering::Relaxed);
        })
        .expect("tray thread");
}
