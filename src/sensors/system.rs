use super::{Kind, Reading};
use std::collections::HashMap;
use wmi::{Variant, WMIConnection};

type Row = HashMap<String, Variant>;

fn number(row: &Row, key: &str) -> i64 {
    match row.get(key) {
        Some(Variant::UI1(v)) => *v as i64,
        Some(Variant::UI2(v)) => *v as i64,
        Some(Variant::UI4(v)) => *v as i64,
        Some(Variant::UI8(v)) => *v as i64,
        Some(Variant::I2(v)) => *v as i64,
        Some(Variant::I4(v)) => *v as i64,
        Some(Variant::I8(v)) => *v,
        _ => 0,
    }
}

fn text(row: &Row, key: &str) -> Option<String> {
    match row.get(key) {
        Some(Variant::String(s)) => Some(s.trim().to_string()),
        _ => None,
    }
}

/// Things Windows has no power sensors for, priced at typical values.
pub fn estimates() -> Vec<Reading> {
    let mut list = Vec::new();

    if let Ok(con) = WMIConnection::new() {
        let rows: Vec<Row> = con.raw_query("SELECT SMBIOSMemoryType FROM Win32_PhysicalMemory").unwrap_or_default();
        let ddr5 = rows.iter().any(|r| number(r, "SMBIOSMemoryType") == 34);
        if !rows.is_empty() {
            list.push(Reading::estimate(
                "Memory",
                format!("{} sticks, {}", rows.len(), if ddr5 { "DDR5" } else { "DDR4 or older" }),
                rows.len() as f32 * if ddr5 { 4.0 } else { 3.0 },
                Kind::Memory,
            ));
        }
    }

    if let Ok(con) = WMIConnection::with_namespace_path(r"ROOT\Microsoft\Windows\Storage") {
        let rows: Vec<Row> = con.raw_query("SELECT FriendlyName, MediaType, BusType FROM MSFT_PhysicalDisk").unwrap_or_default();
        for r in &rows {
            let (bus, media) = (number(r, "BusType"), number(r, "MediaType"));
            let (watts, kind) = match (bus, media) {
                (17, _) => (4.0, "NVMe"),
                (_, 3) => (6.0, "HDD"),
                _ => (2.5, "SSD"),
            };
            let name = text(r, "FriendlyName").unwrap_or_else(|| "Disk".into());
            list.push(Reading::estimate("Storage", format!("{name} ({kind})"), watts, Kind::Storage));
        }
    }

    list.push(Reading::estimate("Motherboard, fans, USB", "typical value".into(), 25.0, Kind::Board));
    list
}
