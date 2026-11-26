use crate::catalog::PowerPlan;

#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub total: f32,
    pub cpu: f32,
    pub clock: f32,
    pub load: f32,
    pub gpu_load: f32,
    pub gpu_memory: f32,
}

const STATIC_CPU_WATTS: f32 = 6.0;
const PCIE_WATTS_PER_LEVEL: f32 = 1.0;

pub fn runtime_hours(station_wh: u32, inverter_efficiency: u32, wall_watts: f32) -> f32 {
    if wall_watts > 0.0 { station_wh as f32 * (inverter_efficiency as f32 / 100.0) / wall_watts } else { 0.0 }
}

pub fn format_duration(hours: f32) -> String {
    if hours <= 0.0 || !hours.is_finite() {
        return "—".into();
    }
    let minutes = (hours * 60.0).round() as i64;
    let (h, m) = (minutes / 60, minutes % 60);
    if h > 0 { format!("{h} h {m:02} min") } else { format!("{m} min") }
}

/// Change in total DC watts if `target` replaced `cur` under the current load.
pub fn delta(s: &Sample, base_clock: f32, peak_clock: f32, cur: &PowerPlan, target: &PowerPlan) -> f32 {
    let cpu = cpu_watts(s, base_clock, peak_clock, cur, target) - s.cpu;
    let pcie = (cur.aspm as f32 - target.aspm as f32) * PCIE_WATTS_PER_LEVEL;
    cpu + pcie
}

// Rough DVFS model: dynamic CPU power ~ f^2.5, a plan only moves the frequency window
fn cpu_watts(s: &Sample, base_clock: f32, peak_clock: f32, cur: &PowerPlan, target: &PowerPlan) -> f32 {
    if s.cpu <= 0.0 || s.clock <= 0.0 || base_clock <= 0.0 {
        return s.cpu;
    }

    let boost = peak_clock.max(base_clock * 1.3);
    let cap = |p: &PowerPlan| {
        if p.turbo_allowed() { boost } else { base_clock * p.max_state.min(99) as f32 / 100.0 }
    };
    let floor = |p: &PowerPlan| (base_clock * p.min_state as f32 / 100.0).min(cap(p));

    let fixed = s.cpu.min(STATIC_CPU_WATTS);
    let dynamic = s.cpu - fixed;

    // Near idle the minimum processor state is what matters: 100% keeps every core at full
    // clocks, 5% lets them drift down. Compare where each plan would park an idle CPU.
    if s.load < 30.0 && cur.min_state != target.min_state {
        let idle = base_clock * (s.load / 100.0 * 2.0).max(0.5);
        let parked = |p: &PowerPlan| idle.max(floor(p)).min(cap(p));
        let ratio = (parked(target) / parked(cur)).clamp(0.0, 1.3);
        return fixed + dynamic * ratio.powf(2.5);
    }

    let demand = if s.clock >= cap(cur) * 0.95 { f32::MAX } else { s.clock };
    let f = demand.max(floor(target)).min(cap(target));
    let ratio = (f / s.clock).min(1.6);
    fixed + dynamic * ratio.powf(2.5)
}
