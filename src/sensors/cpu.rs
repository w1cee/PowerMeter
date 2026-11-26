use super::pawnio::PawnIo;
use crate::win;
use std::arch::x86_64::__cpuid;
use std::time::Instant;
use windows::Win32::System::Performance::*;
use windows::core::w;

static AMD_MODULE: &[u8] = include_bytes!("../../assets/pawnio/AMDFamily17.bin");
static INTEL_MODULE: &[u8] = include_bytes!("../../assets/pawnio/IntelMSR.bin");

const CPU_KEY: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    Intel,
    AmdZen,
    Unsupported,
}

pub fn vendor() -> Vendor {
    let leaf0 = __cpuid(0);
    let mut id = [0u8; 12];
    id[..4].copy_from_slice(&leaf0.ebx.to_le_bytes());
    id[4..8].copy_from_slice(&leaf0.edx.to_le_bytes());
    id[8..].copy_from_slice(&leaf0.ecx.to_le_bytes());

    let eax = __cpuid(1).eax;
    let base = (eax >> 8) & 0xF;
    let family = if base == 0xF { base + ((eax >> 20) & 0xFF) } else { base };

    match &id {
        b"GenuineIntel" => Vendor::Intel,
        b"AuthenticAMD" | b"HygonGenuine" if (0x17..=0x1A).contains(&family) => Vendor::AmdZen,
        _ => Vendor::Unsupported,
    }
}

pub fn name() -> String {
    win::reg_string(win::hklm(), CPU_KEY, "ProcessorNameString").map(|s| s.trim().to_string()).unwrap_or_else(|| "CPU".into())
}

pub fn base_mhz() -> f32 {
    win::reg_dword(win::hklm(), CPU_KEY, "~MHz").unwrap_or(0) as f32
}

/// Package power from the RAPL energy counter (Intel and AMD Zen share the layout).
pub struct Rapl {
    io: PawnIo,
    energy_msr: u32,
    joules_per_tick: f64,
    last: Option<(u32, Instant)>,
}

impl Rapl {
    pub fn open(vendor: Vendor) -> Option<Self> {
        let (module, unit_msr, energy_msr) = match vendor {
            Vendor::Intel => (INTEL_MODULE, 0x606, 0x611),
            Vendor::AmdZen => (AMD_MODULE, 0xC001_0299, 0xC001_029B),
            Vendor::Unsupported => return None,
        };
        let io = PawnIo::open(module)?;
        let esu = (io.read_msr(unit_msr)? >> 8) & 0x1F;
        Some(Self { io, energy_msr, joules_per_tick: 0.5f64.powi(esu as i32), last: None })
    }

    pub fn watts(&mut self) -> Option<f32> {
        let raw = self.io.read_msr(self.energy_msr)? as u32;
        let now = Instant::now();
        let result = self.last.and_then(|(prev, at)| {
            let dt = now.duration_since(at).as_secs_f64();
            (dt > 0.05).then(|| (raw.wrapping_sub(prev) as f64 * self.joules_per_tick / dt) as f32)
        });
        self.last = Some((raw, now));
        result
    }
}

/// Load and effective clock the same way Task Manager gets them.
pub struct CpuCounters {
    query: PDH_HQUERY,
    utility: PDH_HCOUNTER,
    performance: PDH_HCOUNTER,
}

unsafe impl Send for CpuCounters {}

impl CpuCounters {
    pub fn open() -> Option<Self> {
        unsafe {
            let mut query = PDH_HQUERY::default();
            if PdhOpenQueryW(None, 0, &mut query) != 0 {
                return None;
            }
            let mut utility = PDH_HCOUNTER::default();
            let mut performance = PDH_HCOUNTER::default();
            PdhAddEnglishCounterW(query, w!(r"\Processor Information(_Total)\% Processor Utility"), 0, &mut utility);
            PdhAddEnglishCounterW(query, w!(r"\Processor Information(_Total)\% Processor Performance"), 0, &mut performance);
            PdhCollectQueryData(query);
            Some(Self { query, utility, performance })
        }
    }

    fn value(counter: PDH_HCOUNTER) -> f32 {
        let mut v = PDH_FMT_COUNTERVALUE::default();
        unsafe {
            if PdhGetFormattedCounterValue(counter, PDH_FMT(PDH_FMT_DOUBLE.0 | 0x8000), None, &mut v) == 0 {
                v.Anonymous.doubleValue as f32
            } else {
                0.0
            }
        }
    }

    /// (load %, performance % of base clock)
    pub fn sample(&mut self) -> (f32, f32) {
        unsafe {
            PdhCollectQueryData(self.query);
        }
        (Self::value(self.utility).clamp(0.0, 100.0), Self::value(self.performance))
    }
}

impl Drop for CpuCounters {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}
