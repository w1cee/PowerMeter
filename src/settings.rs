use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const FILE_NAME: &str = "PowerMeter.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub psu_efficiency: u32,
    pub calibration: f32,
    pub station_wh: u32,
    pub inverter_efficiency: u32,
    pub tariff: f32,
    pub currency: String,
    pub minimize_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            psu_efficiency: 88,
            calibration: 0.0,
            station_wh: 0,
            inverter_efficiency: 85,
            tariff: 4.32,
            currency: "UAH".into(),
            minimize_to_tray: true,
        }
    }
}

// portable first: next to the exe, %APPDATA% only if that folder is read-only
fn portable_path() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|d| d.join(FILE_NAME))
}

fn appdata_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("PowerMeter").join(FILE_NAME))
}

impl Settings {
    pub fn load() -> Self {
        for path in [portable_path(), appdata_path()].into_iter().flatten() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(s) = serde_json::from_str(&text) {
                    return s;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(path) = portable_path() {
            if std::fs::write(&path, &text).is_ok() {
                return Ok(path);
            }
        }
        let path = appdata_path().ok_or("APPDATA is not set")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        Ok(path)
    }

    pub fn location() -> String {
        match (portable_path(), appdata_path()) {
            (Some(p), _) if p.exists() => p.display().to_string(),
            (_, Some(a)) if a.exists() => a.display().to_string(),
            (Some(p), _) => p.display().to_string(),
            _ => "nowhere, apparently".into(),
        }
    }

    pub fn efficiency(&self) -> f32 {
        self.psu_efficiency.clamp(50, 99) as f32 / 100.0
    }
}
