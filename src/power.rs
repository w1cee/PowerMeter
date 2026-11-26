pub use crate::catalog::PowerPlan;
use crate::catalog::{self, PlanId, Template};
use crate::win;
use std::os::windows::process::CommandExt;
use std::process::Command;
use windows::Win32::Foundation::{ERROR_ACCESS_DISABLED_BY_POLICY, HLOCAL, LocalFree, WIN32_ERROR};
use windows::Win32::System::Power::*;
use windows::Win32::System::Registry::RegDeleteKeyValueW;
use windows::core::{GUID, HSTRING};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const POLICY_KEY: &str = r"SOFTWARE\Policies\Microsoft\Power\PowerSettings";

const PROCESSOR: GUID = GUID::from_u128(0x54533251_82be_4824_96c1_47b60b740d00);
const MAX_STATE: GUID = GUID::from_u128(0xbc5038f7_23e0_4960_96da_33abaf5935ec);
const MIN_STATE: GUID = GUID::from_u128(0x893dee8e_2bef_41e0_89c6_b55d0929964c);
const BOOST_MODE: GUID = GUID::from_u128(0xbe337238_0d82_4146_a960_4f3749d470c7);
const EPP: GUID = GUID::from_u128(0x36687f9e_e3a5_4dbf_b1dc_15eb381c6863);
const PCIE: GUID = GUID::from_u128(0x501a4d13_42af_4429_9fd1_a8218c268e20);
const ASPM: GUID = GUID::from_u128(0xee12f906_d277_404b_b6da_e5fa1a576df5);
const USB: GUID = GUID::from_u128(0x2a737441_1930_4402_8d77_b2bebba308a3);
const USB_SUSPEND: GUID = GUID::from_u128(0x48e6b7a6_50f5_4782_a5d4_53bb8f07e226);

fn guid(id: PlanId) -> GUID {
    GUID::from_u128(id.0)
}

#[derive(Debug)]
pub enum PowerError {
    Policy,
    Win32(u32),
    Other(String),
}

impl std::fmt::Display for PowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PowerError::Policy => {
                write!(f, "The active plan is pinned by a group policy. Hit \"Unlock\" at the top and try again.")
            }
            PowerError::Win32(code) => write!(f, "{}", windows::core::Error::from(WIN32_ERROR(*code).to_hresult()).message()),
            PowerError::Other(s) => write!(f, "{s}"),
        }
    }
}

fn check(err: WIN32_ERROR) -> Result<(), PowerError> {
    match err {
        e if e.0 == 0 => Ok(()),
        ERROR_ACCESS_DISABLED_BY_POLICY => Err(PowerError::Policy),
        e => Err(PowerError::Win32(e.0)),
    }
}

fn on_battery() -> bool {
    let mut status = windows::Win32::System::Power::SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status).is_ok() && status.ACLineStatus == 0 }
}

pub fn load() -> Vec<PowerPlan> {
    let active = active_scheme();
    let battery = on_battery();
    let mut plans = Vec::new();

    for index in 0.. {
        let mut id = GUID::zeroed();
        let mut size = size_of::<GUID>() as u32;
        let err =
            unsafe { PowerEnumerate(None, None, None, ACCESS_SCHEME, index, Some(&mut id as *mut GUID as *mut u8), &mut size) };
        if err.0 != 0 {
            break;
        }
        let read = |group: GUID, setting: GUID, fallback: u32| read_value(id, group, setting, battery).unwrap_or(fallback);
        plans.push(PowerPlan {
            id: PlanId(id.to_u128()),
            name: friendly_name(id),
            max_state: read(PROCESSOR, MAX_STATE, 100),
            min_state: read(PROCESSOR, MIN_STATE, 5),
            boost_mode: read(PROCESSOR, BOOST_MODE, 1),
            aspm: read(PCIE, ASPM, 0),
            active: Some(id) == active,
        });
    }
    plans
}

fn active_scheme() -> Option<GUID> {
    let mut ptr: *mut GUID = std::ptr::null_mut();
    unsafe {
        if PowerGetActiveScheme(None, &mut ptr).0 != 0 || ptr.is_null() {
            return None;
        }
        let id = *ptr;
        let _ = LocalFree(Some(HLOCAL(ptr as _)));
        Some(id)
    }
}

fn friendly_name(id: GUID) -> String {
    unsafe {
        let mut size = 0u32;
        let _ = PowerReadFriendlyName(None, Some(&id), None, None, None, &mut size);
        if size == 0 {
            return format!("{id:?}");
        }
        let mut buf = vec![0u16; size as usize / 2 + 1];
        if PowerReadFriendlyName(None, Some(&id), None, None, Some(buf.as_mut_ptr() as *mut u8), &mut size).0 != 0 {
            return format!("{id:?}");
        }
        win::from_wide(&buf)
    }
}

fn read_value(scheme: GUID, group: GUID, setting: GUID, battery: bool) -> Option<u32> {
    let mut value = 0u32;
    let ok = unsafe {
        if battery {
            PowerReadDCValueIndex(None, Some(&scheme), Some(&group), Some(&setting), &mut value) == 0
        } else {
            PowerReadACValueIndex(None, Some(&scheme), Some(&group), Some(&setting), &mut value).0 == 0
        }
    };
    ok.then_some(value)
}

fn write_value(scheme: GUID, group: GUID, setting: GUID, value: u32) {
    unsafe {
        let _ = PowerWriteACValueIndex(None, &scheme, Some(&group), Some(&setting), value);
        let _ = PowerWriteDCValueIndex(None, &scheme, Some(&group), Some(&setting), value);
    }
}

fn utf16_bytes(text: &str) -> Vec<u8> {
    win::wide(text).iter().flat_map(|c| c.to_le_bytes()).collect()
}

pub fn set_active(id: PlanId) -> Result<(), PowerError> {
    check(unsafe { PowerSetActiveScheme(None, Some(&guid(id))) })
}

pub fn install(t: &Template) -> Result<(), PowerError> {
    powercfg(&["-duplicatescheme", &guid_str(catalog::BALANCED), &guid_str(t.id)])?;
    let id = guid(t.id);
    check(unsafe { PowerWriteFriendlyName(None, &id, None, None, &utf16_bytes(t.name)) })?;
    check(unsafe { PowerWriteDescription(None, &id, None, None, &utf16_bytes(t.description)) })?;
    write_value(id, PROCESSOR, MAX_STATE, t.max_state);
    write_value(id, PROCESSOR, MIN_STATE, t.min_state);
    write_value(id, PROCESSOR, BOOST_MODE, t.boost_mode);
    write_value(id, PROCESSOR, EPP, t.epp);
    write_value(id, PCIE, ASPM, t.aspm);
    write_value(id, USB, USB_SUSPEND, 1);
    Ok(())
}

/// Duplicating a built-in scheme onto its own GUID brings it back.
pub fn restore(id: PlanId) -> Result<(), PowerError> {
    powercfg(&["-duplicatescheme", &guid_str(id), &guid_str(id)])
}

pub fn delete(id: PlanId) -> Result<(), PowerError> {
    check(unsafe { PowerDeleteScheme(None, &guid(id)) })
}

fn guid_str(id: PlanId) -> String {
    format!("{:?}", guid(id)).to_lowercase()
}

fn powercfg(args: &[&str]) -> Result<(), PowerError> {
    let status = Command::new("powercfg.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| PowerError::Other(format!("Couldn't start powercfg: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(PowerError::Other(format!("powercfg {} failed with code {}", args.join(" "), status.code().unwrap_or(-1))))
    }
}

pub fn locked_scheme() -> Option<String> {
    win::reg_string(win::hklm(), POLICY_KEY, "ActivePowerScheme")
}

pub fn unlock() -> Result<(), String> {
    unsafe {
        let err = RegDeleteKeyValueW(win::hklm(), &HSTRING::from(POLICY_KEY), &HSTRING::from("ActivePowerScheme"));
        if err.0 != 0 && err.0 != 2 {
            return Err(windows::core::Error::from(err.to_hresult()).message());
        }
    }
    let _ = Command::new("gpupdate.exe").args(["/target:computer", "/force"]).creation_flags(CREATE_NO_WINDOW).status();
    Ok(())
}
