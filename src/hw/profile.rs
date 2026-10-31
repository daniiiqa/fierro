//! Power profiles. Where the firmware has an ACPI platform_profile (ThinkPad, ASUS,
//! IdeaPad, …) it sets fans and power limits; the CPU's governor, EPP and turbo are set
//! too, like power-profiles-daemon does. Without one (Huawei MateBooks) the CPU side
//! is the whole profile.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::{Error, Result, Sysfs};

const CPUFREQ: &str = "sys/devices/system/cpu/cpufreq";
const NO_TURBO: &str = "sys/devices/system/cpu/intel_pstate/no_turbo";
/// AMD and acpi-cpufreq's turbo switch, the other way around from no_turbo.
const BOOST: &str = "sys/devices/system/cpu/cpufreq/boost";
const PLATFORM: &str = "sys/firmware/acpi/platform_profile";
const PLATFORM_CHOICES: &str = "sys/firmware/acpi/platform_profile_choices";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Ahorro,
    Balance,
    Fierro,
}

impl Profile {
    pub const ALL: [Self; 3] = [Self::Ahorro, Self::Balance, Self::Fierro];

    pub fn name(self) -> &'static str {
        match self {
            Self::Ahorro => "ahorro",
            Self::Balance => "balance",
            Self::Fierro => "fierro",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Ahorro => Self::Balance,
            Self::Balance => Self::Fierro,
            Self::Fierro => Self::Ahorro,
        }
    }

    fn governor(self) -> &'static str {
        match self {
            Self::Fierro => "performance",
            _ => "powersave",
        }
    }

    fn epp(self) -> &'static str {
        match self {
            Self::Ahorro => "power",
            Self::Balance => "balance_performance",
            Self::Fierro => "performance",
        }
    }

    fn turbo(self) -> bool {
        self != Self::Ahorro
    }

    /// platform_profile names for this profile, best first.
    fn platform_names(self) -> &'static [&'static str] {
        match self {
            Self::Ahorro => &["low-power", "quiet", "cool"],
            Self::Balance => &["balanced"],
            Self::Fierro => &["performance", "balanced-performance"],
        }
    }
}

impl FromStr for Profile {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.name() == s)
            .ok_or_else(|| Error::Invalid(format!("unknown profile {s:?} (ahorro, balance, fierro)")))
    }
}

/// What the machine is actually set to; `profile` is None when it matches none of ours.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileState {
    pub profile: Option<Profile>,
    /// "platform_profile" or "cpufreq": what decides the profile.
    pub driver: &'static str,
    pub platform: Option<String>,
    pub governor: Option<String>,
    pub epp: Option<String>,
    pub turbo: Option<bool>,
}

fn policies(fs: &Sysfs) -> Vec<String> {
    fs.list(CPUFREQ, "policy").into_iter().map(|p| format!("{CPUFREQ}/{p}")).collect()
}

fn read_turbo(fs: &Sysfs) -> Option<bool> {
    fs.parse_opt::<u8>(NO_TURBO).map(|v| v == 0).or_else(|| fs.parse_opt::<u8>(BOOST).map(|v| v == 1))
}

pub fn read(fs: &Sysfs) -> Result<ProfileState> {
    let platform = fs.read_opt(PLATFORM);
    let policy = policies(fs).into_iter().next();
    if platform.is_none() && policy.is_none() {
        return Err(Error::Unsupported("power profiles"));
    }
    let governor = policy.as_ref().and_then(|p| fs.read_opt(&format!("{p}/scaling_governor")));
    let epp = policy.as_ref().and_then(|p| fs.read_opt(&format!("{p}/energy_performance_preference")));
    let turbo = read_turbo(fs);
    let (driver, profile) = match &platform {
        Some(pp) => ("platform_profile", Profile::ALL.into_iter().find(|p| p.platform_names().contains(&pp.as_str()))),
        None => (
            "cpufreq",
            Profile::ALL.into_iter().find(|p| {
                governor.as_deref() == Some(p.governor())
                    && epp.as_deref().is_none_or(|e| e == p.epp())
                    && turbo.is_none_or(|t| t == p.turbo())
            }),
        ),
    };
    Ok(ProfileState { profile, driver, platform, governor, epp, turbo })
}

pub fn apply(fs: &Sysfs, profile: Profile) -> Result<()> {
    let policies = policies(fs);
    if !fs.exists(PLATFORM) && policies.is_empty() {
        return Err(Error::Unsupported("power profiles"));
    }
    if fs.exists(PLATFORM) {
        let choices = fs.read(PLATFORM_CHOICES).unwrap_or_default();
        let name = profile
            .platform_names()
            .iter()
            .find(|n| choices.split_whitespace().any(|c| c == **n))
            .ok_or_else(|| Error::Invalid(format!("platform_profile has no {} level ({choices})", profile.name())))?;
        fs.write(PLATFORM, name)?;
    }
    for policy in policies {
        // Governor first: intel_pstate refuses EPP changes while on `performance`.
        fs.write(&format!("{policy}/scaling_governor"), profile.governor())?;
        let epp = format!("{policy}/energy_performance_preference");
        if fs.exists(&epp) {
            fs.write(&epp, profile.epp())?;
        }
    }
    if fs.exists(NO_TURBO) {
        fs.write(NO_TURBO, if profile.turbo() { "0" } else { "1" })?;
    } else if fs.exists(BOOST) {
        fs.write(BOOST, if profile.turbo() { "1" } else { "0" })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::fake;

    fn cpu() -> Sysfs {
        fake(&[
            ("sys/devices/system/cpu/cpufreq/policy0/scaling_governor", "powersave"),
            ("sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference", "power"),
            ("sys/devices/system/cpu/cpufreq/policy1/scaling_governor", "powersave"),
            ("sys/devices/system/cpu/cpufreq/policy1/energy_performance_preference", "power"),
            ("sys/devices/system/cpu/intel_pstate/no_turbo", "1"),
        ])
    }

    #[test]
    fn detects_and_applies() {
        let fs = cpu();
        assert_eq!(read(&fs).unwrap().profile, Some(Profile::Ahorro));
        apply(&fs, Profile::Fierro).unwrap();
        let state = read(&fs).unwrap();
        assert_eq!(state.profile, Some(Profile::Fierro));
        assert_eq!(state.turbo, Some(true));
        assert_eq!(fs.read("sys/devices/system/cpu/cpufreq/policy1/scaling_governor").unwrap(), "performance");
    }

    #[test]
    fn unknown_mix_is_custom() {
        let fs = cpu();
        fs.write("sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference", "default").unwrap();
        assert_eq!(read(&fs).unwrap().profile, None);
    }

    #[test]
    fn platform_profile_decides() {
        let fs = fake(&[
            ("sys/firmware/acpi/platform_profile", "balanced"),
            ("sys/firmware/acpi/platform_profile_choices", "quiet balanced performance"),
            ("sys/devices/system/cpu/cpufreq/policy0/scaling_governor", "powersave"),
            ("sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference", "balance_performance"),
            ("sys/devices/system/cpu/cpufreq/boost", "1"),
        ]);
        let state = read(&fs).unwrap();
        assert_eq!((state.driver, state.profile), ("platform_profile", Some(Profile::Balance)));
        // ASUS calls its low-power level "quiet"
        apply(&fs, Profile::Ahorro).unwrap();
        assert_eq!(fs.read("sys/firmware/acpi/platform_profile").unwrap(), "quiet");
        assert_eq!(fs.read("sys/devices/system/cpu/cpufreq/boost").unwrap(), "0");
        assert_eq!(read(&fs).unwrap().profile, Some(Profile::Ahorro));
    }

    #[test]
    fn missing_platform_level_is_an_error() {
        let fs = fake(&[
            ("sys/firmware/acpi/platform_profile", "balanced"),
            ("sys/firmware/acpi/platform_profile_choices", "balanced performance"),
        ]);
        assert!(matches!(apply(&fs, Profile::Ahorro), Err(Error::Invalid(_))));
    }
}
