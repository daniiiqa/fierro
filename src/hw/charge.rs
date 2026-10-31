//! Battery charge limits: Huawei's "Smart Charge", the start/end thresholds of
//! ThinkPads and most other laptops, end-only thresholds (ASUS, Framework, Samsung),
//! and Lenovo IdeaPad's on/off conservation mode.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::battery::POWER_SUPPLY;
use super::{ideapad_dir, Error, Result, Sysfs, WMI_DIR};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thresholds {
    pub start: u8,
    pub end: u8,
}

impl Thresholds {
    pub const FULL: Self = Self { start: 0, end: 100 };

    pub fn new(start: u8, end: u8) -> Result<Self> {
        if end == 0 || end > 100 || start >= end {
            return Err(Error::Invalid(format!(
                "invalid charge limits {start}–{end}: need 0 ≤ start < end ≤ 100"
            )));
        }
        Ok(Self { start, end })
    }

    pub fn is_limited(&self) -> bool {
        self.end < 100
    }
}

impl fmt::Display for Thresholds {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}–{}%", self.start, self.end)
    }
}

/// What the firmware lets us set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Where charging resumes and where it stops.
    Range,
    /// Only where it stops.
    EndOnly,
    /// On or off; the firmware picks the level.
    Toggle,
}

/// Nominal level of IdeaPad conservation mode; the firmware decides (55–80% by model).
pub const CONSERVATION_END: u8 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    /// Plugged in all day: keep the cells in their happy range.
    Casa,
    Oficina,
    /// Top it up before leaving.
    Viaje,
}

impl Preset {
    pub const ALL: [Self; 3] = [Self::Casa, Self::Oficina, Self::Viaje];

    pub fn name(self) -> &'static str {
        match self {
            Self::Casa => "casa",
            Self::Oficina => "oficina",
            Self::Viaje => "viaje",
        }
    }

    pub fn thresholds(self) -> Thresholds {
        match self {
            Self::Casa => Thresholds { start: 40, end: 70 },
            Self::Oficina => Thresholds { start: 70, end: 90 },
            Self::Viaje => Thresholds::FULL,
        }
    }

    /// The presets a kind of control can tell apart: on/off has no "oficina".
    pub fn for_kind(kind: Kind) -> &'static [Self] {
        match kind {
            Kind::Toggle => &[Self::Casa, Self::Viaje],
            _ => &Self::ALL,
        }
    }

    pub fn matching(t: Thresholds) -> Option<Self> {
        // Huawei firmware stores "0 100" as "95 100": any 100% end is viaje.
        if !t.is_limited() {
            return Some(Self::Viaje);
        }
        Self::ALL.into_iter().find(|p| p.thresholds() == t)
    }

    pub fn matching_for(kind: Kind, t: Thresholds) -> Option<Self> {
        match kind {
            Kind::Range => Self::matching(t),
            Kind::EndOnly => Self::ALL.into_iter().find(|p| p.thresholds().end == t.end),
            Kind::Toggle => Some(if t.is_limited() { Self::Casa } else { Self::Viaje }),
        }
    }
}

impl FromStr for Preset {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.name() == s)
            .ok_or_else(|| Error::Invalid(format!("unknown preset {s:?} (casa, oficina, viaje)")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Iface {
    /// huawei-wmi: one file, "START END".
    Combined(String),
    Pair { start: String, end: String },
    End(String),
    /// ideapad_laptop conservation_mode: 0 or 1.
    Conservation(String),
}

/// The charge-limit interface this machine has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    pub driver: &'static str,
    iface: Iface,
}

impl Control {
    pub fn detect(fs: &Sysfs) -> Result<Self> {
        let wmi = format!("{WMI_DIR}/charge_control_thresholds");
        if fs.exists(&wmi) {
            return Ok(Self { driver: "huawei-wmi", iface: Iface::Combined(wmi) });
        }
        if let Some(dev) = ideapad_dir(fs) {
            let path = format!("{dev}/conservation_mode");
            if fs.exists(&path) {
                return Ok(Self { driver: "ideapad_laptop", iface: Iface::Conservation(path) });
            }
        }
        for bat in fs.list(POWER_SUPPLY, "BAT") {
            let start = format!("{POWER_SUPPLY}/{bat}/charge_control_start_threshold");
            let end = format!("{POWER_SUPPLY}/{bat}/charge_control_end_threshold");
            match (fs.exists(&start), fs.exists(&end)) {
                (true, true) => return Ok(Self { driver: "battery", iface: Iface::Pair { start, end } }),
                (false, true) => return Ok(Self { driver: "battery", iface: Iface::End(end) }),
                _ => {}
            }
        }
        Err(Error::Unsupported("charge limits"))
    }

    pub fn kind(&self) -> Kind {
        match self.iface {
            Iface::Combined(_) | Iface::Pair { .. } => Kind::Range,
            Iface::End(_) => Kind::EndOnly,
            Iface::Conservation(_) => Kind::Toggle,
        }
    }

    pub fn read(&self, fs: &Sysfs) -> Result<Thresholds> {
        match &self.iface {
            Iface::Combined(path) => {
                let raw = fs.read(path)?;
                let mut parts = raw.split_whitespace().map(str::parse::<u8>);
                match (parts.next(), parts.next()) {
                    // Some firmwares report "0 0" when limits are off.
                    (Some(Ok(_)), Some(Ok(0))) => Ok(Thresholds::FULL),
                    (Some(Ok(start)), Some(Ok(end))) => Ok(Thresholds { start, end }),
                    _ => Err(Error::Parse { path: fs.path(path), value: raw }),
                }
            }
            Iface::Pair { start, end } => Ok(Thresholds { start: fs.parse(start)?, end: fs.parse(end)? }),
            Iface::End(end) => Ok(Thresholds { start: 0, end: fs.parse(end)? }),
            Iface::Conservation(path) => Ok(if fs.parse::<u8>(path)? != 0 {
                Thresholds { start: 0, end: CONSERVATION_END }
            } else {
                Thresholds::FULL
            }),
        }
    }

    /// Writes what the interface supports: end-only ignores `start`, on/off is on
    /// for any limit below 100%.
    pub fn write(&self, fs: &Sysfs, t: Thresholds) -> Result<()> {
        match &self.iface {
            Iface::Combined(path) => fs.write(path, &format!("{} {}\n", t.start, t.end)),
            Iface::Pair { start, end } => {
                // The kernel rejects start >= end after each write, so order them around the current end.
                let current_end: u8 = fs.parse(end)?;
                if t.start >= current_end {
                    fs.write(end, &t.end.to_string())?;
                    fs.write(start, &t.start.to_string())
                } else {
                    fs.write(start, &t.start.to_string())?;
                    fs.write(end, &t.end.to_string())
                }
            }
            Iface::End(end) => fs.write(end, &t.end.to_string()),
            Iface::Conservation(path) => fs.write(path, if t.is_limited() { "1" } else { "0" }),
        }
    }

    pub fn preset(&self, t: Thresholds) -> Option<Preset> {
        Preset::matching_for(self.kind(), t)
    }

    pub fn describe(&self, t: Thresholds) -> String {
        describe(self.kind(), t)
    }
}

/// "40–70%", "up to 70%" or "conservation on".
pub fn describe(kind: Kind, t: Thresholds) -> String {
    match kind {
        Kind::Range => t.to_string(),
        Kind::EndOnly => format!("up to {}%", t.end),
        Kind::Toggle => format!("conservation {}", if t.is_limited() { "on" } else { "off" }),
    }
}

pub fn read(fs: &Sysfs) -> Result<Thresholds> {
    Control::detect(fs)?.read(fs)
}

pub fn write(fs: &Sysfs, t: Thresholds) -> Result<()> {
    Control::detect(fs)?.write(fs, t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::fake;

    #[test]
    fn validates() {
        assert!(Thresholds::new(40, 70).is_ok());
        assert!(Thresholds::new(0, 100).is_ok());
        assert!(Thresholds::new(70, 70).is_err());
        assert!(Thresholds::new(80, 70).is_err());
        assert!(Thresholds::new(0, 101).is_err());
    }

    #[test]
    fn round_trips_through_huawei_wmi() {
        let fs = fake(&[("sys/devices/platform/huawei-wmi/charge_control_thresholds", "95 100\n")]);
        assert_eq!(Control::detect(&fs).unwrap().driver, "huawei-wmi");
        assert_eq!(read(&fs).unwrap(), Thresholds { start: 95, end: 100 });
        write(&fs, Preset::Casa.thresholds()).unwrap();
        assert_eq!(Preset::matching(read(&fs).unwrap()), Some(Preset::Casa));
    }

    #[test]
    fn firmware_full_is_viaje() {
        assert_eq!(Preset::matching(Thresholds { start: 95, end: 100 }), Some(Preset::Viaje));
        assert_eq!(Preset::matching(Thresholds { start: 50, end: 80 }), None);
    }

    #[test]
    fn zero_means_full() {
        let fs = fake(&[("sys/devices/platform/huawei-wmi/charge_control_thresholds", "0 0")]);
        assert_eq!(read(&fs).unwrap(), Thresholds::FULL);
    }

    #[test]
    fn falls_back_to_battery_attrs() {
        let fs = fake(&[
            ("sys/class/power_supply/BAT0/charge_control_start_threshold", "40"),
            ("sys/class/power_supply/BAT0/charge_control_end_threshold", "70"),
        ]);
        assert_eq!(Control::detect(&fs).unwrap().kind(), Kind::Range);
        write(&fs, Thresholds { start: 80, end: 90 }).unwrap();
        assert_eq!(read(&fs).unwrap(), Thresholds { start: 80, end: 90 });
    }

    #[test]
    fn end_only_ignores_start() {
        let fs = fake(&[("sys/class/power_supply/BAT0/charge_control_end_threshold", "100")]);
        let c = Control::detect(&fs).unwrap();
        assert_eq!(c.kind(), Kind::EndOnly);
        c.write(&fs, Preset::Casa.thresholds()).unwrap();
        let t = c.read(&fs).unwrap();
        assert_eq!(t, Thresholds { start: 0, end: 70 });
        assert_eq!(c.preset(t), Some(Preset::Casa));
        assert_eq!(c.describe(t), "up to 70%");
    }

    #[test]
    fn ideapad_conservation_is_a_toggle() {
        let fs = fake(&[
            ("sys/bus/platform/drivers/ideapad_acpi/VPC2004:00/conservation_mode", "0"),
            ("sys/class/power_supply/BAT0/charge_control_end_threshold", "100"),
        ]);
        let c = Control::detect(&fs).unwrap();
        assert_eq!((c.driver, c.kind()), ("ideapad_laptop", Kind::Toggle));
        c.write(&fs, Preset::Oficina.thresholds()).unwrap();
        assert_eq!(fs.read("sys/bus/platform/drivers/ideapad_acpi/VPC2004:00/conservation_mode").unwrap(), "1");
        assert_eq!(c.preset(c.read(&fs).unwrap()), Some(Preset::Casa));
        c.write(&fs, Preset::Viaje.thresholds()).unwrap();
        assert_eq!(c.describe(c.read(&fs).unwrap()), "conservation off");
    }
}
