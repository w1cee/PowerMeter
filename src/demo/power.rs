pub use crate::catalog::PowerPlan;
use crate::catalog::{self, PlanId, Template};
use std::sync::Mutex;

#[derive(Debug)]
pub struct PowerError(String);

impl std::fmt::Display for PowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

static PLANS: Mutex<Vec<PowerPlan>> = Mutex::new(Vec::new());

fn plan(id: u128, name: &str, max: u32, min: u32, boost: u32, aspm: u32, active: bool) -> PowerPlan {
    PowerPlan { id: PlanId(id), name: name.into(), max_state: max, min_state: min, boost_mode: boost, aspm, active }
}

pub fn load() -> Vec<PowerPlan> {
    let mut plans = PLANS.lock().unwrap();
    if plans.is_empty() {
        *plans = vec![
            plan(catalog::BALANCED.0, "Balanced", 100, 5, 1, 1, false),
            plan(0x3ce5b344_3077_4544_9205_fa356cf1e14b, "Ultimate Performance", 100, 100, 2, 0, true),
            plan(0x9897998c_92de_4669_853f_b7cd3ecb2790, "AMD Ryzen™ Balanced", 100, 90, 1, 1, false),
        ];
    }
    plans.clone()
}

pub fn set_active(id: PlanId) -> Result<(), PowerError> {
    for p in PLANS.lock().unwrap().iter_mut() {
        p.active = p.id == id;
    }
    Ok(())
}

pub fn install(t: &Template) -> Result<(), PowerError> {
    PLANS.lock().unwrap().push(t.to_plan());
    Ok(())
}

pub fn restore(id: PlanId) -> Result<(), PowerError> {
    let s = catalog::STANDARD.iter().find(|s| s.id == id).ok_or(PowerError("unknown plan".into()))?;
    PLANS.lock().unwrap().push(plan(id.0, s.name, 100, 5, 1, 1, false));
    Ok(())
}

pub fn delete(id: PlanId) -> Result<(), PowerError> {
    PLANS.lock().unwrap().retain(|p| p.id != id);
    Ok(())
}

pub fn locked_scheme() -> Option<String> {
    None
}

pub fn unlock() -> Result<(), String> {
    Ok(())
}
