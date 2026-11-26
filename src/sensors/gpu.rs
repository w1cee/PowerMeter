use super::{GpuSample, GpuVendor};
use std::ffi::c_void;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{LMEM_ZEROINIT, LocalAlloc};
use windows::core::{PCSTR, w};

pub trait Gpu: Send {
    fn sample(&mut self) -> Option<GpuSample>;
}

pub fn open() -> Option<Box<dyn Gpu>> {
    if let Some(n) = Nvml::open() {
        return Some(Box::new(n));
    }
    if let Some(a) = Adl::open() {
        return Some(Box::new(a));
    }
    None
}

unsafe fn proc<T>(lib: HMODULE, name: &str) -> Option<T> {
    let cname = format!("{name}\0");
    let p = unsafe { GetProcAddress(lib, PCSTR(cname.as_ptr()))? };
    Some(unsafe { std::mem::transmute_copy(&p) })
}

fn c_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

// ---------- NVIDIA: NVML ships with the driver ----------

type NvmlInit = unsafe extern "C" fn() -> i32;
type NvmlCount = unsafe extern "C" fn(*mut u32) -> i32;
type NvmlHandle = unsafe extern "C" fn(u32, *mut *mut c_void) -> i32;
type NvmlName = unsafe extern "C" fn(*mut c_void, *mut u8, u32) -> i32;
type NvmlPower = unsafe extern "C" fn(*mut c_void, *mut u32) -> i32;
type NvmlClock = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32;
type NvmlUtil = unsafe extern "C" fn(*mut c_void, *mut [u32; 2]) -> i32;

struct Nvml {
    device: *mut c_void,
    name: String,
    power: NvmlPower,
    clock: NvmlClock,
    util: NvmlUtil,
}

unsafe impl Send for Nvml {}

impl Nvml {
    fn open() -> Option<Self> {
        unsafe {
            let lib = LoadLibraryW(w!("nvml.dll")).ok()?;
            let init: NvmlInit = proc(lib, "nvmlInit_v2")?;
            let count: NvmlCount = proc(lib, "nvmlDeviceGetCount_v2")?;
            let handle: NvmlHandle = proc(lib, "nvmlDeviceGetHandleByIndex_v2")?;
            let get_name: NvmlName = proc(lib, "nvmlDeviceGetName")?;
            if init() != 0 {
                return None;
            }
            let mut n = 0;
            if count(&mut n) != 0 || n == 0 {
                return None;
            }
            let mut device = std::ptr::null_mut();
            if handle(0, &mut device) != 0 {
                return None;
            }
            let mut buf = [0u8; 96];
            get_name(device, buf.as_mut_ptr(), buf.len() as u32);
            Some(Self {
                device,
                name: c_string(&buf),
                power: proc(lib, "nvmlDeviceGetPowerUsage")?,
                clock: proc(lib, "nvmlDeviceGetClockInfo")?,
                util: proc(lib, "nvmlDeviceGetUtilizationRates")?,
            })
        }
    }
}

impl Gpu for Nvml {
    fn sample(&mut self) -> Option<GpuSample> {
        unsafe {
            let mut mw = 0u32;
            let power = ((self.power)(self.device, &mut mw) == 0).then_some(mw as f32 / 1000.0);
            let (mut core, mut mem) = (0u32, 0u32);
            (self.clock)(self.device, 0, &mut core);
            (self.clock)(self.device, 2, &mut mem);
            let mut util = [0u32; 2];
            (self.util)(self.device, &mut util);
            Some(GpuSample {
                name: self.name.clone(),
                vendor: GpuVendor::Nvidia,
                power,
                load: util[0] as f32,
                core_clock: core as f32,
                memory_clock: mem as f32,
            })
        }
    }
}

// ---------- AMD: ADL PMLog, same source Adrenalin's overlay uses ----------

const ADL_MAX_PATH: usize = 256;
const PMLOG_MAX_SENSORS: usize = 256;
const PMLOG_CLK_GFXCLK: usize = 1;
const PMLOG_CLK_MEMCLK: usize = 2;
const PMLOG_INFO_ACTIVITY_GFX: usize = 19;
const PMLOG_ASIC_POWER: usize = 23;
const PMLOG_BOARD_POWER: usize = 73;

#[repr(C)]
struct AdapterInfo {
    size: i32,
    adapter_index: i32,
    udid: [u8; ADL_MAX_PATH],
    bus_number: i32,
    device_number: i32,
    function_number: i32,
    vendor_id: i32,
    adapter_name: [u8; ADL_MAX_PATH],
    display_name: [u8; ADL_MAX_PATH],
    present: i32,
    exist: i32,
    driver_path: [u8; ADL_MAX_PATH],
    driver_path_ext: [u8; ADL_MAX_PATH],
    pnp_string: [u8; ADL_MAX_PATH],
    os_display_index: i32,
}

#[repr(C)]
struct PmLogOutput {
    size: i32,
    sensors: [[i32; 2]; PMLOG_MAX_SENSORS],
}

type AdlAlloc = unsafe extern "system" fn(i32) -> *mut c_void;
type AdlCreate = unsafe extern "system" fn(AdlAlloc, i32, *mut *mut c_void) -> i32;
type AdlCount = unsafe extern "system" fn(*mut c_void, *mut i32) -> i32;
type AdlInfo = unsafe extern "system" fn(*mut c_void, *mut AdapterInfo, i32) -> i32;
type AdlActive = unsafe extern "system" fn(*mut c_void, i32, *mut i32) -> i32;
type AdlPmLog = unsafe extern "system" fn(*mut c_void, i32, *mut PmLogOutput) -> i32;

fn is_amd(a: &AdapterInfo) -> bool {
    let udid = c_string(&a.udid).to_ascii_uppercase();
    a.vendor_id == 0x1002 || udid.contains("VEN_1002")
}

unsafe extern "system" fn adl_alloc(size: i32) -> *mut c_void {
    unsafe { LocalAlloc(LMEM_ZEROINIT, size.max(0) as usize).map(|h| h.0).unwrap_or(std::ptr::null_mut()) }
}

struct Adl {
    context: *mut c_void,
    adapter: i32,
    name: String,
    pmlog: AdlPmLog,
}

unsafe impl Send for Adl {}

impl Adl {
    fn open() -> Option<Self> {
        unsafe {
            let lib = LoadLibraryW(w!("atiadlxx.dll")).ok()?;
            let create: AdlCreate = proc(lib, "ADL2_Main_Control_Create")?;
            let count: AdlCount = proc(lib, "ADL2_Adapter_NumberOfAdapters_Get")?;
            let info: AdlInfo = proc(lib, "ADL2_Adapter_AdapterInfo_Get")?;
            let active: AdlActive = proc(lib, "ADL2_Adapter_Active_Get")?;
            let pmlog: AdlPmLog = proc(lib, "ADL2_New_QueryPMLogData_Get")?;

            let mut context = std::ptr::null_mut();
            if create(adl_alloc, 1, &mut context) != 0 {
                return None;
            }
            let mut n = 0;
            if count(context, &mut n) != 0 || n <= 0 {
                return None;
            }
            let mut adapters: Vec<AdapterInfo> = (0..n).map(|_| std::mem::zeroed()).collect();
            for a in adapters.iter_mut() {
                a.size = size_of::<AdapterInfo>() as i32;
            }
            if info(context, adapters.as_mut_ptr(), (size_of::<AdapterInfo>() * adapters.len()) as i32) != 0 {
                return None;
            }

            // ADL mangles VendorID on Windows, the UDID ("PCI_VEN_1002&...") is the reliable source.
            // Every card shows up once per output, so try them all, active ones first.
            let mut candidates: Vec<&AdapterInfo> = adapters.iter().filter(|a| is_amd(a)).collect();
            candidates.sort_by_key(|a| {
                let mut is_active = 0;
                active(context, a.adapter_index, &mut is_active);
                is_active == 0
            });
            for a in candidates {
                let mut out: PmLogOutput = std::mem::zeroed();
                if pmlog(context, a.adapter_index, &mut out) == 0 {
                    return Some(Self { context, adapter: a.adapter_index, name: c_string(&a.adapter_name), pmlog });
                }
            }
            None
        }
    }
}

impl Gpu for Adl {
    fn sample(&mut self) -> Option<GpuSample> {
        let mut out: PmLogOutput = unsafe { std::mem::zeroed() };
        if unsafe { (self.pmlog)(self.context, self.adapter, &mut out) } != 0 {
            return None;
        }
        let get = |i: usize| (out.sensors[i][0] != 0).then_some(out.sensors[i][1] as f32);
        // RDNA3+ report the whole board, older cards only the ASIC
        let power = get(PMLOG_BOARD_POWER).filter(|&w| w > 0.0).or_else(|| get(PMLOG_ASIC_POWER));
        Some(GpuSample {
            name: self.name.clone(),
            vendor: GpuVendor::Amd,
            power,
            load: get(PMLOG_INFO_ACTIVITY_GFX).unwrap_or(0.0),
            core_clock: get(PMLOG_CLK_GFXCLK).unwrap_or(0.0),
            memory_clock: get(PMLOG_CLK_MEMCLK).unwrap_or(0.0),
        })
    }
}
