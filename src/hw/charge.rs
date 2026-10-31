//! Battery charge limits — Huawei's "Smart Charge".

use std::fmt;
use std::str::FromStr;

use serde::Serialize;

use super::battery::POWER_SUPPLY;
use super::{Error, Result, Sysfs, WMI_DIR};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

    pub fn matching(t: Thresholds) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.thresholds() == t)
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

fn wmi_path() -> String {
    format!("{WMI_DIR}/charge_control_thresholds")
}

/// Generic fallback for firmwares without huawei-wmi thresholds.
fn battery_dir(fs: &Sysfs) -> Result<String> {
    fs.list(POWER_SUPPLY, "BAT")
        .into_iter()
        .map(|b| format!("{POWER_SUPPLY}/{b}"))
        .find(|dir| fs.exists(&format!("{dir}/charge_control_end_threshold")))
        .ok_or(Error::Unsupported("charge limits"))
}

pub fn read(fs: &Sysfs) -> Result<Thresholds> {
    let wmi = wmi_path();
    if fs.exists(&wmi) {
        let raw = fs.read(&wmi)?;
        let mut parts = raw.split_whitespace().map(str::parse::<u8>);
        return match (parts.next(), parts.next()) {
            // Some firmwares report "0 0" when limits are off.
            (Some(Ok(_)), Some(Ok(0))) => Ok(Thresholds::FULL),
            (Some(Ok(start)), Some(Ok(end))) => Ok(Thresholds { start, end }),
            _ => Err(Error::Parse { path: fs.path(&wmi), value: raw }),
        };
    }

    let bat = battery_dir(fs)?;
    Ok(Thresholds {
        start: fs.parse(&format!("{bat}/charge_control_start_threshold"))?,
        end: fs.parse(&format!("{bat}/charge_control_end_threshold"))?,
    })
}

pub fn write(fs: &Sysfs, t: Thresholds) -> Result<()> {
    let wmi = wmi_path();
    if fs.exists(&wmi) {
        return fs.write(&wmi, &format!("{} {}\n", t.start, t.end));
    }

    let bat = battery_dir(fs)?;
    let start = format!("{bat}/charge_control_start_threshold");
    let end = format!("{bat}/charge_control_end_threshold");
    // The kernel rejects start >= end after each write, so order them around the current end.
    let current_end: u8 = fs.parse(&end)?;
    if t.start >= current_end {
        fs.write(&end, &t.end.to_string())?;
        fs.write(&start, &t.start.to_string())
    } else {
        fs.write(&start, &t.start.to_string())?;
        fs.write(&end, &t.end.to_string())
    }
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
        assert_eq!(read(&fs).unwrap(), Thresholds { start: 95, end: 100 });
        write(&fs, Preset::Casa.thresholds()).unwrap();
        assert_eq!(Preset::matching(read(&fs).unwrap()), Some(Preset::Casa));
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
        write(&fs, Thresholds { start: 80, end: 90 }).unwrap();
        assert_eq!(read(&fs).unwrap(), Thresholds { start: 80, end: 90 });
    }
}
