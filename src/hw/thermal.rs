use serde::Serialize;

use super::Sysfs;

const HWMON: &str = "sys/class/hwmon";

#[derive(Debug, Clone, Serialize)]
pub struct Sensor {
    pub chip: String,
    pub label: String,
    pub celsius: f64,
    pub max: Option<f64>,
    pub crit: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Fan {
    pub chip: String,
    pub label: String,
    pub rpm: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Thermals {
    pub sensors: Vec<Sensor>,
    pub fans: Vec<Fan>,
}

impl Thermals {
    pub fn read(fs: &Sysfs) -> Self {
        let mut out = Self::default();
        for hw in fs.list(HWMON, "hwmon") {
            let dir = format!("{HWMON}/{hw}");
            let chip = fs.read_opt(&format!("{dir}/name")).unwrap_or_else(|| hw.clone());
            let label = |base: &str| fs.read_opt(&format!("{dir}/{base}_label")).unwrap_or_else(|| chip.clone());
            let milli = |base: &str, attr: &str| fs.parse_opt::<f64>(&format!("{dir}/{base}_{attr}")).map(|v| v / 1000.0);

            for base in inputs(fs, &dir, "temp") {
                if let Some(celsius) = milli(&base, "input") {
                    out.sensors.push(Sensor {
                        chip: chip.clone(),
                        label: label(&base),
                        celsius,
                        max: milli(&base, "max"),
                        crit: milli(&base, "crit"),
                    });
                }
            }
            for base in inputs(fs, &dir, "fan") {
                if let Some(rpm) = fs.parse_opt(&format!("{dir}/{base}_input")) {
                    out.fans.push(Fan { chip: chip.clone(), label: label(&base), rpm });
                }
            }
        }
        out
    }

    /// Best single "CPU temperature" number for bars and the dashboard.
    pub fn cpu(&self) -> Option<f64> {
        let find = |chip: &str, prefix: &str| {
            self.sensors.iter().find(|s| s.chip == chip && s.label.starts_with(prefix)).map(|s| s.celsius)
        };
        find("coretemp", "Package").or_else(|| find("k10temp", "Tctl")).or_else(|| find("acpitz", ""))
    }
}

/// `temp1`, `temp2`, … for every `<kind>N_input` in `dir`.
fn inputs(fs: &Sysfs, dir: &str, kind: &str) -> Vec<String> {
    fs.list(dir, kind)
        .into_iter()
        .filter_map(|f| f.strip_suffix("_input").map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::fake;

    #[test]
    fn reads_sensors_and_fans() {
        let fs = fake(&[
            ("sys/class/hwmon/hwmon1/name", "acpi_fan"),
            ("sys/class/hwmon/hwmon1/fan1_input", "2400"),
            ("sys/class/hwmon/hwmon5/name", "coretemp"),
            ("sys/class/hwmon/hwmon5/temp1_input", "48000"),
            ("sys/class/hwmon/hwmon5/temp1_label", "Package id 0"),
            ("sys/class/hwmon/hwmon5/temp1_crit", "100000"),
            ("sys/class/hwmon/hwmon5/temp2_input", "45000"),
            ("sys/class/hwmon/hwmon5/temp2_label", "Core 0"),
        ]);
        let t = Thermals::read(&fs);
        assert_eq!(t.cpu(), Some(48.0));
        assert_eq!(t.sensors.len(), 2);
        assert_eq!(t.sensors[0].crit, Some(100.0));
        assert_eq!(t.fans[0].rpm, 2400);
    }
}
