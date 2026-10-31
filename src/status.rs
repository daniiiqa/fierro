//! One snapshot of everything fierro can see, shared by the CLI, the bar and the TUI.

use serde::Serialize;

use crate::hw::battery::Battery;
use crate::hw::charge::{self, Preset, Thresholds};
use crate::hw::profile::{self, ProfileState};
use crate::hw::thermal::Thermals;
use crate::hw::{fnlock, led, Machine, Sysfs};

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub machine: Machine,
    pub battery: Option<Battery>,
    pub charge: Option<Thresholds>,
    pub preset: Option<Preset>,
    pub fnlock: Option<bool>,
    pub profile: Option<ProfileState>,
    pub micmute_led: Option<bool>,
    pub cpu_celsius: Option<f64>,
    /// Plugged in but not charging because Smart Charge reached its limit.
    pub held: bool,
    pub thermals: Thermals,
}

impl Status {
    pub fn read(fs: &Sysfs) -> Self {
        let battery = Battery::read(fs).ok();
        let charge = charge::read(fs).ok();
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
            preset: charge.and_then(Preset::matching),
            battery,
            charge,
            fnlock: fnlock::read(fs).ok(),
            profile: profile::read(fs).ok(),
            micmute_led: led::micmute(fs).ok(),
            cpu_celsius: thermals.cpu(),
            held,
            thermals,
        }
    }
}
