/// Power scheme GUID as a plain number, so the UI doesn't care about the platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlanId(pub u128);

#[derive(Clone, Debug)]
pub struct PowerPlan {
    pub id: PlanId,
    pub name: String,
    pub max_state: u32,
    pub min_state: u32,
    pub boost_mode: u32,
    pub aspm: u32,
    pub active: bool,
}

impl PowerPlan {
    pub fn turbo_allowed(&self) -> bool {
        self.boost_mode != 0 && self.max_state >= 100
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlanKind {
    Windows,
    Ours,
    Other,
}

pub struct Template {
    pub id: PlanId,
    pub name: &'static str,
    pub description: &'static str,
    pub max_state: u32,
    pub min_state: u32,
    pub boost_mode: u32,
    pub aspm: u32,
    pub epp: u32,
}

impl Template {
    pub fn to_plan(&self) -> PowerPlan {
        PowerPlan {
            id: self.id,
            name: self.name.into(),
            max_state: self.max_state,
            min_state: self.min_state,
            boost_mode: self.boost_mode,
            aspm: self.aspm,
            active: false,
        }
    }
}

pub struct Standard {
    pub id: PlanId,
    pub name: &'static str,
    pub description: &'static str,
    /// localized names seen in the wild, for plans duplicated with a random GUID
    pub aliases: &'static [&'static str],
}

pub const BALANCED: PlanId = PlanId(0x381b4222_f694_41f0_9685_ff5bb260df2e);

pub const STANDARD: &[Standard] = &[
    Standard {
        id: BALANCED,
        name: "Balanced",
        description: "Windows default: clocks follow the load, turbo on",
        aliases: &["Balanced", "Сбалансированная", "Збалансований", "Збалансована"],
    },
    Standard {
        id: PlanId(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c),
        name: "High performance",
        description: "Less power saving, the CPU happily sits at high clocks",
        aliases: &["High performance", "Высокая производительность", "Висока продуктивність"],
    },
    Standard {
        id: PlanId(0xa1841308_3541_4fab_bc81_f71556f20b4a),
        name: "Power saver",
        description: "Windows' own saver; barely does anything on a desktop",
        aliases: &["Power saver", "Экономия энергии", "Економія енергії"],
    },
    Standard {
        id: PlanId(0xe9a42b02_d5df_448d_aa00_03f14749eb61),
        name: "Ultimate Performance",
        description: "Power saving switched off entirely. Your power station hates this one",
        aliases: &["Ultimate Performance", "Максимальная производительность", "Максимальна продуктивність"],
    },
];

pub const OURS: &[Template] = &[
    Template {
        id: PlanId(0xb7e3a1c0_5d2f_4e8a_9c61_0f4d2a7e5b01),
        name: "PowerMeter · Smart turbo",
        description: "Turbo stays, but in efficient mode; idles properly",
        max_state: 100,
        min_state: 5,
        boost_mode: 3,
        aspm: 1,
        epp: 33,
    },
    Template {
        id: PlanId(0xb7e3a1c0_5d2f_4e8a_9c61_0f4d2a7e5b02),
        name: "PowerMeter · Eco",
        description: "No turbo. You won't feel it in daily use, the meter will",
        max_state: 99,
        min_state: 5,
        boost_mode: 0,
        aspm: 1,
        epp: 50,
    },
    Template {
        id: PlanId(0xb7e3a1c0_5d2f_4e8a_9c61_0f4d2a7e5b03),
        name: "PowerMeter · Quiet",
        description: "CPU capped at 80%, max PCIe saving: browser, code, video",
        max_state: 80,
        min_state: 5,
        boost_mode: 0,
        aspm: 2,
        epp: 70,
    },
    Template {
        id: PlanId(0xb7e3a1c0_5d2f_4e8a_9c61_0f4d2a7e5b04),
        name: "PowerMeter · Blackout",
        description: "CPU capped at 50%: torrents, music and waiting for the grid",
        max_state: 50,
        min_state: 5,
        boost_mode: 0,
        aspm: 2,
        epp: 90,
    },
];

pub fn find_standard(p: &PowerPlan) -> Option<&'static Standard> {
    STANDARD.iter().find(|s| s.id == p.id || s.aliases.iter().any(|a| a.eq_ignore_ascii_case(&p.name) || *a == p.name))
}

pub fn find_ours(id: PlanId) -> Option<&'static Template> {
    OURS.iter().find(|t| t.id == id)
}

pub fn kind_of(p: &PowerPlan) -> PlanKind {
    if find_ours(p.id).is_some() {
        PlanKind::Ours
    } else if find_standard(p).is_some() {
        PlanKind::Windows
    } else {
        PlanKind::Other
    }
}
