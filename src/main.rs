#![windows_subsystem = "windows"]
#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

mod app;
mod catalog;
mod estimator;
mod sensors;
mod settings;
mod theme;
mod ui;

// the real thing talks to Windows; elsewhere the same API is faked for UI work
#[cfg(windows)]
mod display;
#[cfg(windows)]
mod pawnio_setup;
#[cfg(windows)]
mod power;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod win;

#[cfg(not(windows))]
#[path = "demo/display.rs"]
mod display;
#[cfg(not(windows))]
#[path = "demo/pawnio_setup.rs"]
mod pawnio_setup;
#[cfg(not(windows))]
#[path = "demo/power.rs"]
mod power;
#[cfg(not(windows))]
#[path = "demo/tray.rs"]
mod tray;
#[cfg(not(windows))]
#[path = "demo/win.rs"]
mod win;

use eframe::egui;

static ICON: &[u8] = include_bytes!("../assets/icon64.rgba");

fn main() -> eframe::Result {
    if !win::single_instance() {
        win::info(None, "PowerMeter is already running. Check the tray, it's probably hiding there.");
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PowerMeter")
            .with_inner_size([1240.0, 900.0])
            .with_min_inner_size([1040.0, 760.0])
            .with_icon(egui::IconData { rgba: ICON.to_vec(), width: 64, height: 64 }),
        centered: true,
        ..Default::default()
    };

    eframe::run_native("PowerMeter", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
