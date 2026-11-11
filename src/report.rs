//! `fierro report`: what this laptop exposes, for a bug report or a new test fixture.
//! Reads only named attributes, so serial numbers never end up in it.

use serde_json::{json, Map, Value};

use crate::hw::battery::POWER_SUPPLY;
use crate::hw::Sysfs;
use crate::status::Status;

const DMI: [&str; 7] =
    ["sys_vendor", "product_name", "product_version", "product_family", "board_vendor", "board_name", "bios_version"];

const SUPPLY_ATTRS: [&str; 12] = [
    "type",
    "status",
    "capacity",
    "charge_control_start_threshold",
    "charge_control_end_threshold",
    "charge_types",
    "charge_behaviour",
    "energy_full",
    "energy_full_design",
    "charge_full",
    "charge_full_design",
    "cycle_count",
];

/// Platform driver directories whose attribute *names* show what a vendor offers.
const PLATFORM_DIRS: [&str; 8] = [
    "sys/devices/platform/huawei-wmi",
    "sys/devices/platform/thinkpad_acpi",
    "sys/devices/platform/asus-nb-wmi",
    "sys/devices/platform/dell-laptop",
    "sys/devices/platform/msi-ec",
    "sys/devices/platform/samsung-galaxybook",
    "sys/devices/platform/framework_laptop",
    "sys/bus/platform/drivers/ideapad_acpi",
];

const MODULE_HINTS: [&str; 7] = ["wmi", "laptop", "acpi", "_ec", "platform", "profile", "charge"];

fn attrs(fs: &Sysfs, dir: &str, names: &[&str]) -> Value {
    let found: Map<String, Value> = names
        .iter()
        .filter_map(|n| fs.read_opt(&format!("{dir}/{n}")).map(|v| (n.to_string(), json!(v))))
        .collect();
    Value::Object(found)
}

pub fn report(fs: &Sysfs) -> Value {
    let supplies: Map<String, Value> =
        fs.list(POWER_SUPPLY, "").into_iter().map(|n| (n.clone(), attrs(fs, &format!("{POWER_SUPPLY}/{n}"), &SUPPLY_ATTRS))).collect();
    let platform: Map<String, Value> = PLATFORM_DIRS
        .iter()
        .filter(|d| fs.exists(d))
        .map(|d| (d.to_string(), json!(fs.list(d, ""))))
        .collect();
    let modules: Vec<String> = fs
        .read_opt("proc/modules")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|m| !m.starts_with("snd") && MODULE_HINTS.iter().any(|h| m.contains(h)))
        .map(String::from)
        .collect();
    let hwmon: Vec<String> = fs
        .list("sys/class/hwmon", "hwmon")
        .iter()
        .filter_map(|h| fs.read_opt(&format!("sys/class/hwmon/{h}/name")))
        .collect();
    let status = Status::read(fs);

    json!({
        "fierro": env!("CARGO_PKG_VERSION"),
        "kernel": fs.read_opt("proc/sys/kernel/osrelease"),
        "dmi": attrs(fs, "sys/class/dmi/id", &DMI),
        "modules": modules,
        "power_supply": supplies,
        "platform_profile": attrs(fs, "sys/firmware/acpi", &["platform_profile", "platform_profile_choices"]),
        "cpufreq": attrs(
            fs,
            "sys/devices/system/cpu/cpufreq/policy0",
            &["scaling_driver", "scaling_available_governors", "energy_performance_available_preferences"],
        ),
        "platform": platform,
        "leds": fs.list("sys/class/leds", ""),
        "hwmon": hwmon,
        "detected": {
            "charge": status.charge_driver.map(|d| json!({ "driver": d, "kind": status.charge_kind })),
            "profile": status.profile.as_ref().map(|p| p.driver),
            "fnlock": status.fnlock_driver,
            "kbd": status.kbd.as_ref().map(|k| k.name.clone()),
            "fingerprint": status.fingerprint,
            "pam_fprintd": crate::hw::fingerprint::pam(fs),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::fake;

    #[test]
    fn leaves_serials_out() {
        let fs = fake(&[
            ("sys/class/dmi/id/sys_vendor", "LENOVO"),
            ("sys/class/dmi/id/product_serial", "SECRET"),
            ("sys/class/power_supply/BAT0/serial_number", "SECRET"),
            ("sys/class/power_supply/BAT0/charge_control_end_threshold", "80"),
            ("proc/modules", "thinkpad_acpi 1 0 - Live 0x0\nsnd 1 0 - Live 0x0\n"),
        ]);
        let r = report(&fs).to_string();
        assert!(!r.contains("SECRET"));
        assert!(r.contains("thinkpad_acpi") && !r.contains("\"snd\""));
        assert!(r.contains("\"kind\":\"end-only\""));
    }
}
