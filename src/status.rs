//! One snapshot of everything fierro can see, shared by the CLI, the bar and the TUI.

use serde::Serialize;

use crate::hw::battery::Battery;
use crate::hw::charge::{self, Control, Kind, Preset, Thresholds};
use crate::hw::kbd::{self, Kbd};
use crate::hw::profile::{self, ProfileState};
use crate::hw::thermal::Thermals;
use crate::hw::fingerprint::{self, Reader};
use crate::hw::{fnlock, led, Machine, Sysfs};

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub machine: Machine,
    pub battery: Option<Battery>,
    pub charge: Option<Thresholds>,
    pub charge_kind: Option<Kind>,
    pub charge_driver: Option<&'static str>,
    pub preset: Option<Preset>,
    pub fnlock: Option<bool>,
    pub fnlock_driver: Option<&'static str>,
    pub profile: Option<ProfileState>,
    pub kbd: Option<Kbd>,
    pub micmute_led: Option<bool>,
    pub cpu_celsius: Option<f64>,
    pub fingerprint: Option<Reader>,
    /// Plugged in but not charging because the charge limit was reached.
    pub held: bool,
    pub thermals: Thermals,
}

impl Status {
    pub fn read(fs: &Sysfs) -> Self {
        let battery = Battery::read(fs).ok();
        let control = Control::detect(fs).ok();
        let charge = control.as_ref().and_then(|c| c.read(fs).ok());
        let thermals = Thermals::read(fs);
        let held = match (&battery, charge) {
            // Not "Discharging": rewriting the limits makes the EC drop charging for a moment.
            (Some(b), Some(t)) => {
                t.is_limited() && b.ac_online == Some(true) && matches!(b.status.as_str(), "Not charging" | "Full")
            }
            _ => false,
        };
        Self {
            machine: Machine::detect(fs),
            preset: control.as_ref().zip(charge).and_then(|(c, t)| c.preset(t)),
            charge_kind: control.as_ref().map(Control::kind),
            charge_driver: control.as_ref().map(|c| c.driver),
            battery,
            charge,
            fnlock: fnlock::read(fs).ok(),
            fnlock_driver: fnlock::control(fs).ok().map(|(_, driver)| driver),
            profile: profile::read(fs).ok(),
            kbd: kbd::read(fs).ok(),
            micmute_led: led::micmute(fs).ok(),
            cpu_celsius: thermals.cpu(),
            fingerprint: fingerprint::detect(fs),
            held,
            thermals,
        }
    }

    /// "40–70% (casa)", "up to 70% (casa)", "conservation on".
    pub fn describe_charge(&self) -> Option<String> {
        let kind = self.charge_kind?;
        let text = charge::describe(kind, self.charge?);
        Some(match self.preset {
            Some(p) if kind != Kind::Toggle => format!("{text} ({})", p.name()),
            _ => text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::profile::Profile;
    use crate::testing::{fake, machines};

    #[test]
    fn huawei_matebook() {
        let s = Status::read(&fake(&machines::huawei()));
        assert_eq!((s.charge_driver, s.charge_kind), (Some("huawei-wmi"), Some(Kind::Range)));
        assert_eq!(s.describe_charge().unwrap(), "40–70% (casa)");
        assert_eq!(s.fnlock_driver, Some("huawei-wmi"));
        assert_eq!(s.profile.unwrap().driver, "cpufreq");
        assert!(s.kbd.is_none() && s.held);
    }

    #[test]
    fn thinkpad() {
        let s = Status::read(&fake(&machines::thinkpad()));
        assert_eq!(s.machine.model(), "ThinkPad X1 Carbon Gen 11");
        assert_eq!(s.describe_charge().unwrap(), "75–80%");
        let p = s.profile.unwrap();
        assert_eq!((p.driver, p.profile), ("platform_profile", Some(Profile::Balance)));
        assert_eq!(s.kbd.unwrap().max, 2);
        assert!(s.fnlock.is_none());
    }

    #[test]
    fn asus_zenbook() {
        let s = Status::read(&fake(&machines::asus()));
        assert_eq!(s.charge_kind, Some(Kind::EndOnly));
        assert_eq!(s.describe_charge().unwrap(), "up to 80%");
        assert_eq!(s.profile.unwrap().profile, Some(Profile::Ahorro));
    }

    #[test]
    fn ideapad() {
        let s = Status::read(&fake(&machines::ideapad()));
        assert_eq!((s.charge_driver, s.charge_kind), (Some("ideapad_laptop"), Some(Kind::Toggle)));
        assert_eq!(s.describe_charge().unwrap(), "conservation on");
        assert_eq!(s.fnlock_driver, Some("ideapad_laptop"));
        let p = s.profile.unwrap();
        assert_eq!((p.profile, p.turbo), (Some(Profile::Ahorro), Some(false)));
    }
}
