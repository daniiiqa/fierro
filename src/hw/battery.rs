use serde::Serialize;

use super::{Error, Result, Sysfs};

pub(crate) const POWER_SUPPLY: &str = "sys/class/power_supply";

#[derive(Debug, Clone, Serialize)]
pub struct Battery {
    pub name: String,
    pub status: String,
    pub capacity: u8,
    pub energy_now_wh: f64,
    pub energy_full_wh: f64,
    pub energy_design_wh: f64,
    pub power_w: f64,
    pub voltage_v: f64,
    pub cycles: Option<u32>,
    pub health_pct: f64,
    pub minutes_left: Option<u32>,
    pub ac_online: Option<bool>,
}

impl Battery {
    pub fn read(fs: &Sysfs) -> Result<Self> {
        let name = fs.list(POWER_SUPPLY, "BAT").into_iter().next().ok_or(Error::Unsupported("battery"))?;
        let attr = |a: &str| format!("{POWER_SUPPLY}/{name}/{a}");
        let micro = |a: &str| fs.parse_opt::<f64>(&attr(a)).map(|v| v / 1e6);

        // Some firmwares report charge (µAh) instead of energy (µWh).
        let voltage_design = micro("voltage_min_design");
        let energy = |kind: &str| {
            micro(&format!("energy_{kind}"))
                .or_else(|| Some(micro(&format!("charge_{kind}"))? * voltage_design?))
                .unwrap_or(0.0)
        };
        let power_w = micro("power_now")
            .or_else(|| Some(micro("current_now")? * micro("voltage_now")?))
            .unwrap_or(0.0);

        let status = fs.read(&attr("status"))?;
        let now = energy("now");
        let full = energy("full");
        let design = energy("full_design");
        let hours = match status.as_str() {
            "Discharging" if power_w > 0.0 => Some(now / power_w),
            "Charging" if power_w > 0.0 => Some((full - now).max(0.0) / power_w),
            _ => None,
        };

        Ok(Self {
            name: name.clone(),
            capacity: fs.parse(&attr("capacity"))?,
            energy_now_wh: now,
            energy_full_wh: full,
            energy_design_wh: design,
            power_w,
            voltage_v: micro("voltage_now").unwrap_or(0.0),
            cycles: fs.parse_opt(&attr("cycle_count")),
            health_pct: if design > 0.0 { full / design * 100.0 } else { 0.0 },
            minutes_left: hours.map(|h| (h * 60.0).round() as u32),
            ac_online: ac_online(fs),
            status,
        })
    }

    pub fn is_charging(&self) -> bool {
        self.status == "Charging"
    }
}

fn ac_online(fs: &Sysfs) -> Option<bool> {
    fs.list(POWER_SUPPLY, "")
        .into_iter()
        .find(|n| fs.read_opt(&format!("{POWER_SUPPLY}/{n}/type")).as_deref() == Some("Mains"))
        .and_then(|n| fs.parse_opt::<u8>(&format!("{POWER_SUPPLY}/{n}/online")))
        .map(|v| v == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::fake;

    #[test]
    fn reads_energy_battery() {
        let fs = fake(&[
            ("sys/class/power_supply/AC0/type", "Mains"),
            ("sys/class/power_supply/AC0/online", "0"),
            ("sys/class/power_supply/BAT0/status", "Discharging"),
            ("sys/class/power_supply/BAT0/capacity", "34"),
            ("sys/class/power_supply/BAT0/energy_now", "18000000"),
            ("sys/class/power_supply/BAT0/energy_full", "51050000"),
            ("sys/class/power_supply/BAT0/energy_full_design", "55237000"),
            ("sys/class/power_supply/BAT0/power_now", "9000000"),
            ("sys/class/power_supply/BAT0/cycle_count", "130"),
        ]);
        let b = Battery::read(&fs).unwrap();
        assert_eq!(b.capacity, 34);
        assert_eq!(b.minutes_left, Some(120));
        assert_eq!(b.ac_online, Some(false));
        assert!((b.health_pct - 92.42).abs() < 0.01);
    }

    #[test]
    fn falls_back_to_charge_units() {
        let fs = fake(&[
            ("sys/class/power_supply/BAT1/status", "Full"),
            ("sys/class/power_supply/BAT1/capacity", "100"),
            ("sys/class/power_supply/BAT1/voltage_min_design", "10000000"),
            ("sys/class/power_supply/BAT1/charge_full", "5000000"),
            ("sys/class/power_supply/BAT1/charge_full_design", "5000000"),
        ]);
        let b = Battery::read(&fs).unwrap();
        assert_eq!(b.energy_full_wh, 50.0);
        assert_eq!(b.health_pct, 100.0);
        assert_eq!(b.minutes_left, None);
    }
}
