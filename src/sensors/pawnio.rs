use crate::win::OwnedHandle;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::w;

const DEVICE_TYPE: u32 = 41394 << 16;
const IOCTL_LOAD_BINARY: u32 = DEVICE_TYPE | (0x821 << 2);
const IOCTL_EXECUTE_FN: u32 = DEVICE_TYPE | (0x841 << 2);
const FN_NAME_LENGTH: usize = 32;

/// One driver handle with one signed PawnIO module loaded into it.
pub struct PawnIo {
    handle: OwnedHandle,
}

impl PawnIo {
    pub fn open(module: &[u8]) -> Option<Self> {
        unsafe {
            let handle = CreateFileW(
                w!(r"\\?\GLOBALROOT\Device\PawnIO"),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
            .ok()?;
            let handle = OwnedHandle(handle);
            DeviceIoControl(handle.0, IOCTL_LOAD_BINARY, Some(module.as_ptr() as _), module.len() as u32, None, 0, None, None)
                .ok()?;
            Some(Self { handle })
        }
    }

    fn execute(&self, name: &str, input: &[u64], output: &mut [u64]) -> Option<usize> {
        let mut buf = vec![0u8; FN_NAME_LENGTH + input.len() * 8];
        let n = name.len().min(FN_NAME_LENGTH - 1);
        buf[..n].copy_from_slice(&name.as_bytes()[..n]);
        for (i, v) in input.iter().enumerate() {
            buf[FN_NAME_LENGTH + i * 8..FN_NAME_LENGTH + i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        let mut read = 0u32;
        unsafe {
            DeviceIoControl(
                self.handle.0,
                IOCTL_EXECUTE_FN,
                Some(buf.as_ptr() as _),
                buf.len() as u32,
                Some(output.as_mut_ptr() as _),
                (output.len() * 8) as u32,
                Some(&mut read),
                None,
            )
            .ok()?;
        }
        Some(read as usize / 8)
    }

    pub fn read_msr(&self, msr: u32) -> Option<u64> {
        let mut out = [0u64; 1];
        match self.execute("ioctl_read_msr", &[msr as u64], &mut out)? {
            1 => Some(out[0]),
            _ => None,
        }
    }
}
