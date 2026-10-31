//! `~/.config/fierro/config.json`: settings, plus what `fierro restore` reapplies at login.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::hw::charge::{self, Thresholds};
use crate::hw::profile::{self, Profile};
use crate::hw::{Error, Result, Sysfs};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// "es" or "en"; empty follows $LANG.
    pub lang: String,
    /// Reapply the saved charge limits and profile on `fierro restore`.
    pub restore: bool,
    pub charge: Option<Thresholds>,
    pub profile: Option<Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Self { lang: String::new(), restore: true, charge: None, profile: None }
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
}

impl Config {
    pub fn path() -> PathBuf {
        config_dir().join("fierro/config.json")
    }

    pub fn load() -> Self {
        fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        let json = serde_json::to_string_pretty(self).expect("config serializes") + "\n";
        fs::create_dir_all(path.parent().expect("config path has a parent"))
            .and_then(|()| fs::write(&path, json))
            .map_err(|source| Error::Io { path, source })
    }

    pub fn spanish(&self) -> bool {
        match self.lang.as_str() {
            "es" => true,
            "en" => false,
            _ => std::env::var("LANG").is_ok_and(|l| l.starts_with("es")),
        }
    }

    /// Applies the saved settings that differ from the hardware; returns what changed.
    /// Unchanged limits are not rewritten: every write makes the EC pause charging.
    pub fn restore(&self, fs: &Sysfs) -> Result<Vec<String>> {
        let mut done = Vec::new();
        if !self.restore {
            return Ok(done);
        }
        if let Some(t) = self.charge {
            let t = Thresholds::new(t.start, t.end)?;
            if charge::read(fs).ok() != Some(t) {
                charge::write(fs, t)?;
                done.push(format!("charge {t}"));
            }
        }
        if let Some(p) = self.profile
            && profile::read(fs).ok().and_then(|s| s.profile) != Some(p) {
                profile::apply(fs, p)?;
                done.push(format!("profile {}", p.name()));
            }
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::fake;

    #[test]
    fn restore_only_writes_what_changed() {
        let fs = fake(&[
            ("sys/devices/platform/huawei-wmi/charge_control_thresholds", "40 70"),
            ("sys/devices/system/cpu/cpufreq/policy0/scaling_governor", "powersave"),
            ("sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference", "power"),
        ]);
        let config = Config {
            charge: Some(Thresholds { start: 40, end: 70 }),
            profile: Some(Profile::Balance),
            ..Config::default()
        };
        assert_eq!(config.restore(&fs).unwrap(), ["profile balance"]);
        assert!(config.restore(&fs).unwrap().is_empty());
        let off = Config { restore: false, ..config };
        fs.write("sys/devices/platform/huawei-wmi/charge_control_thresholds", "70 90").unwrap();
        assert!(off.restore(&fs).unwrap().is_empty());
    }
}
