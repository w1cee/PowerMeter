use std::collections::BTreeSet;
use windows::Win32::Devices::Display::{SDC_APPLY, SDC_TOPOLOGY_EXTEND, SetDisplayConfig};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::core::{BOOL, PCWSTR};

#[derive(Clone, Debug, Default)]
pub struct DisplayInfo {
    pub screens: usize,
    pub width: u32,
    pub height: u32,
    pub hz: u32,
    pub rates: Vec<u32>,
}

struct Monitor {
    device: Vec<u16>,
    primary: bool,
}

fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool() {
            let end = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
            let mut device = info.szDevice[..end].to_vec();
            device.push(0);
            list.push(Monitor { device, primary: info.monitorInfo.dwFlags & 1 != 0 });
        }
        true.into()
    }

    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

fn new_mode() -> DEVMODEW {
    DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() }
}

fn primary_device() -> Option<Vec<u16>> {
    monitors().into_iter().find(|m| m.primary).map(|m| m.device)
}

pub fn read() -> DisplayInfo {
    let all = monitors();
    let Some(primary) = all.iter().find(|m| m.primary) else {
        return DisplayInfo { screens: all.len(), ..Default::default() };
    };
    let name = PCWSTR(primary.device.as_ptr());

    let mut cur = new_mode();
    unsafe {
        let _ = EnumDisplaySettingsW(name, ENUM_CURRENT_SETTINGS, &mut cur);
    }

    let mut found = BTreeSet::new();
    let mut i = 0;
    loop {
        let mut mode = new_mode();
        if !unsafe { EnumDisplaySettingsW(name, ENUM_DISPLAY_SETTINGS_MODE(i), &mut mode) }.as_bool() {
            break;
        }
        if mode.dmPelsWidth == cur.dmPelsWidth && mode.dmPelsHeight == cur.dmPelsHeight {
            found.insert(mode.dmDisplayFrequency);
        }
        i += 1;
    }

    // 59/119/143 are the fractional twins of 60/120/144
    let rates = found.iter().copied().filter(|&r| r >= 60 && !found.contains(&(r + 1))).collect();

    DisplayInfo { screens: all.len(), width: cur.dmPelsWidth, height: cur.dmPelsHeight, hz: cur.dmDisplayFrequency, rates }
}

pub fn set_refresh(hz: u32) -> Result<(), String> {
    let device = primary_device().ok_or("Primary display not found")?;
    let name = PCWSTR(device.as_ptr());
    let mut mode = new_mode();
    unsafe {
        if !EnumDisplaySettingsW(name, ENUM_CURRENT_SETTINGS, &mut mode).as_bool() {
            return Err("Couldn't read the current display mode".into());
        }
        mode.dmDisplayFrequency = hz;
        mode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
        let r = ChangeDisplaySettingsExW(name, Some(&mode), None, CDS_UPDATEREGISTRY, None);
        if r != DISP_CHANGE_SUCCESSFUL {
            return Err(format!("Windows refused {hz} Hz (code {})", r.0));
        }
    }
    Ok(())
}

pub fn keep_primary_only() -> Result<(), String> {
    unsafe {
        for m in monitors().into_iter().filter(|m| !m.primary) {
            let mut mode = new_mode();
            mode.dmFields = DM_POSITION | DM_PELSWIDTH | DM_PELSHEIGHT;
            let _ =
                ChangeDisplaySettingsExW(PCWSTR(m.device.as_ptr()), Some(&mode), None, CDS_UPDATEREGISTRY | CDS_NORESET, None);
        }
        let r = ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None);
        if r != DISP_CHANGE_SUCCESSFUL {
            return Err(format!("Couldn't detach the other displays (code {})", r.0));
        }
    }
    Ok(())
}

pub fn extend_all() -> Result<(), String> {
    let r = unsafe { SetDisplayConfig(None, None, SDC_APPLY | SDC_TOPOLOGY_EXTEND) };
    if r != 0 {
        return Err(windows::core::Error::from(windows::core::HRESULT::from_win32(r as u32)).message());
    }
    Ok(())
}
