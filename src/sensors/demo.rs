//! Fake hardware for non-Windows builds: lets you poke at the UI (and take README screenshots)
//! without a single real sensor.

use super::{AVERAGE_WINDOW, CpuStatus, GpuSample, GpuVendor, Kind, Reading, Shared, Snapshot, average};
use crate::estimator::Sample;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

pub fn spawn(shared: Arc<Shared>, repaint: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let start = Instant::now();
        let mut window = VecDeque::new();
        let mut session_wh = 0f64;
        for seq in 1.. {
            let t = start.elapsed().as_secs_f32();
            let wobble = |amp: f32, speed: f32| amp * (t * speed).sin();
            let cpu = 24.0 + wobble(3.0, 0.7) + wobble(1.5, 2.3);
            let gpu = 27.0 + wobble(1.0, 0.4);
            let settings = shared.settings.lock().unwrap().clone();

            let mut readings = vec![
                Reading {
                    component: "AMD Ryzen 5 2600 Six-Core Processor".into(),
                    sensor: "Package (RAPL)".into(),
                    watts: cpu,
                    estimated: false,
                    kind: Kind::Cpu,
                },
                Reading {
                    component: "AMD Radeon RX 9060 XT".into(),
                    sensor: "Board power".into(),
                    watts: gpu,
                    estimated: false,
                    kind: Kind::Gpu,
                },
                Reading::estimate("Memory", "2 sticks, DDR4 or older".into(), 6.0, Kind::Memory),
                Reading::estimate("Storage", "Samsung SSD 980 1TB (NVMe)".into(), 4.0, Kind::Storage),
                Reading::estimate("Motherboard, fans, USB", "typical value".into(), 25.0, Kind::Board),
            ];
            if settings.calibration != 0.0 {
                readings.push(Reading::estimate(
                    "Calibration offset",
                    "fitted to your wall meter".into(),
                    settings.calibration,
                    Kind::Calibration,
                ));
            }
            let total: f32 = readings.iter().map(|r| r.watts).sum();

            if shared.reset_average.swap(false, Ordering::Relaxed) {
                window.clear();
            }
            window.push_back(Sample {
                total,
                cpu,
                clock: 3650.0 + wobble(40.0, 1.1),
                load: 6.0 + wobble(2.0, 0.9),
                gpu_load: 3.0,
                gpu_memory: 2518.0,
            });
            while window.len() > AVERAGE_WINDOW {
                window.pop_front();
            }
            session_wh += (total / settings.efficiency()) as f64 / 3600.0 * 40.0;

            *shared.snapshot.lock().unwrap() = Snapshot {
                seq,
                readings,
                avg: average(&window),
                cpu_name: "AMD Ryzen 5 2600 Six-Core Processor".into(),
                cpu_status: CpuStatus::Ok,
                base_clock: 3400.0,
                peak_clock: 3675.0,
                gpu: Some(GpuSample {
                    name: "AMD Radeon RX 9060 XT".into(),
                    vendor: GpuVendor::Amd,
                    power: Some(gpu),
                    load: 3.0,
                    core_clock: 41.0,
                    memory_clock: 2518.0,
                }),
                session_wh,
            };
            repaint();
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}
