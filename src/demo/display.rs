use std::sync::Mutex;

#[derive(Clone, Debug, Default)]
pub struct DisplayInfo {
    pub screens: usize,
    pub width: u32,
    pub height: u32,
    pub hz: u32,
    pub rates: Vec<u32>,
}

static STATE: Mutex<(usize, u32)> = Mutex::new((2, 165));

pub fn read() -> DisplayInfo {
    let (screens, hz) = *STATE.lock().unwrap();
    DisplayInfo { screens, width: 2560, height: 1440, hz, rates: vec![60, 100, 120, 144, 165] }
}

pub fn set_refresh(hz: u32) -> Result<(), String> {
    STATE.lock().unwrap().1 = hz;
    Ok(())
}

pub fn keep_primary_only() -> Result<(), String> {
    STATE.lock().unwrap().0 = 1;
    Ok(())
}

pub fn extend_all() -> Result<(), String> {
    STATE.lock().unwrap().0 = 2;
    Ok(())
}
