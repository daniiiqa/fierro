//! Power profiles. This model has no ACPI platform_profile, so a profile is
//! the cpufreq governor + Intel EPP + turbo, applied to every policy.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::{Error, Result, Sysfs};

const CPUFREQ: &str = "sys/devices/system/cpu/cpufreq";
const NO_TURBO: &str = "sys/devices/system/cpu/intel_pstate/no_turbo";

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

/// What the CPU is actually set to; `profile` is None when it matches none of ours.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileState {
    pub profile: Option<Profile>,
    pub governor: String,
    pub epp: Option<String>,
    pub turbo: Option<bool>,
}

fn policies(fs: &Sysfs) -> Result<Vec<String>> {
    let policies: Vec<String> = fs.list(CPUFREQ, "policy").into_iter().map(|p| format!("{CPUFREQ}/{p}")).collect();
    if policies.is_empty() {
        return Err(Error::Unsupported("cpufreq"));
    }
    Ok(policies)
}

pub fn read(fs: &Sysfs) -> Result<ProfileState> {
    let policy = &policies(fs)?[0];
    let governor = fs.read(&format!("{policy}/scaling_governor"))?;
    let epp = fs.read_opt(&format!("{policy}/energy_performance_preference"));
    let turbo = fs.parse_opt::<u8>(NO_TURBO).map(|v| v == 0);
    let profile = Profile::ALL.into_iter().find(|p| {
        p.governor() == governor
            && epp.as_deref().is_none_or(|e| e == p.epp())
            && turbo.is_none_or(|t| t == p.turbo())
    });
    Ok(ProfileState { profile, governor, epp, turbo })
}

pub fn apply(fs: &Sysfs, profile: Profile) -> Result<()> {
    for policy in policies(fs)? {
        // Governor first: intel_pstate refuses EPP changes while on `performance`.
        fs.write(&format!("{policy}/scaling_governor"), profile.governor())?;
        let epp = format!("{policy}/energy_performance_preference");
        if fs.exists(&epp) {
            fs.write(&epp, profile.epp())?;
        }
    }
    if fs.exists(NO_TURBO) {
        fs.write(NO_TURBO, if profile.turbo() { "0" } else { "1" })?;
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
}
