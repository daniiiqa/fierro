//! Fake sysfs trees for tests, in this crate and the binary's. Not part of the API.

use std::fs;

use crate::hw::Sysfs;

/// A throwaway sysfs tree holding `files`.
pub fn fake(files: &[(&str, &str)]) -> Sysfs {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);

    let root = std::env::temp_dir().join(format!(
        "fierro-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for (rel, value) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
    }
    Sysfs::at(root)
}

/// Whole fake machines, from each kernel driver's documented sysfs layout.
pub mod machines {
    pub type Files = Vec<(&'static str, &'static str)>;

    fn common() -> Files {
        vec![
            ("sys/class/power_supply/AC/type", "Mains"),
            ("sys/class/power_supply/AC/online", "1"),
            ("sys/class/power_supply/BAT0/type", "Battery"),
            ("sys/class/power_supply/BAT0/status", "Not charging"),
            ("sys/class/power_supply/BAT0/capacity", "80"),
            ("sys/class/power_supply/BAT0/energy_now", "40000000"),
            ("sys/class/power_supply/BAT0/energy_full", "50000000"),
            ("sys/class/power_supply/BAT0/energy_full_design", "57000000"),
            ("sys/devices/system/cpu/cpufreq/policy0/scaling_governor", "powersave"),
            ("sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference", "balance_performance"),
            ("sys/class/hwmon/hwmon0/name", "coretemp"),
            ("sys/class/hwmon/hwmon0/temp1_input", "45000"),
            ("sys/class/hwmon/hwmon0/temp1_label", "Package id 0"),
        ]
    }

    pub fn huawei() -> Files {
        let mut f = common();
        f.extend([
            ("sys/class/dmi/id/sys_vendor", "HUAWEI"),
            ("sys/class/dmi/id/product_name", "MDF-XX"),
            ("sys/devices/platform/huawei-wmi/charge_control_thresholds", "40 70"),
            ("sys/devices/platform/huawei-wmi/fn_lock_state", "0"),
            ("sys/devices/system/cpu/intel_pstate/no_turbo", "0"),
        ]);
        f
    }

    pub fn thinkpad() -> Files {
        let mut f = common();
        f.extend([
            ("sys/class/dmi/id/sys_vendor", "LENOVO"),
            ("sys/class/dmi/id/product_name", "21HM0042US"),
            ("sys/class/dmi/id/product_version", "ThinkPad X1 Carbon Gen 11"),
            ("sys/class/power_supply/BAT0/charge_control_start_threshold", "75"),
            ("sys/class/power_supply/BAT0/charge_control_end_threshold", "80"),
            ("sys/firmware/acpi/platform_profile", "balanced"),
            ("sys/firmware/acpi/platform_profile_choices", "low-power balanced performance"),
            ("sys/class/leds/tpacpi::kbd_backlight/brightness", "1"),
            ("sys/class/leds/tpacpi::kbd_backlight/max_brightness", "2"),
        ]);
        f
    }

    pub fn asus() -> Files {
        let mut f = common();
        f.extend([
            ("sys/class/dmi/id/sys_vendor", "ASUSTeK COMPUTER INC."),
            ("sys/class/dmi/id/product_name", "Zenbook UX3402ZA"),
            ("sys/class/power_supply/BAT0/charge_control_end_threshold", "80"),
            ("sys/firmware/acpi/platform_profile", "quiet"),
            ("sys/firmware/acpi/platform_profile_choices", "quiet balanced performance"),
            ("sys/class/leds/asus::kbd_backlight/brightness", "3"),
            ("sys/class/leds/asus::kbd_backlight/max_brightness", "3"),
        ]);
        f
    }

    pub fn ideapad() -> Files {
        let mut f = common();
        f.extend([
            ("sys/class/dmi/id/sys_vendor", "LENOVO"),
            ("sys/class/dmi/id/product_name", "82KU"),
            ("sys/class/dmi/id/product_version", "IdeaPad 5 14ALC05"),
            ("sys/bus/platform/drivers/ideapad_acpi/VPC2004:00/conservation_mode", "1"),
            ("sys/bus/platform/drivers/ideapad_acpi/VPC2004:00/fn_lock", "0"),
            ("sys/firmware/acpi/platform_profile", "low-power"),
            ("sys/firmware/acpi/platform_profile_choices", "low-power balanced performance"),
            ("sys/devices/system/cpu/cpufreq/boost", "0"),
        ]);
        f
    }
}

