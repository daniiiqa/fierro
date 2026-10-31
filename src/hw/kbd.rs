//! Keyboard backlight: whichever LED the platform driver names `*::kbd_backlight`
//! (tpacpi::, asus::, platform::, dell::…).

use serde::Serialize;

use super::{Error, Result, Sysfs};

const LEDS: &str = "sys/class/leds";

#[derive(Debug, Clone, Serialize)]
pub struct Kbd {
    pub name: String,
    pub level: u32,
    pub max: u32,
}

fn led(fs: &Sysfs) -> Result<String> {
    fs.list(LEDS, "")
        .into_iter()
        .find(|n| n.ends_with("kbd_backlight"))
        .map(|n| format!("{LEDS}/{n}"))
        .ok_or(Error::Unsupported("keyboard backlight"))
}

pub fn read(fs: &Sysfs) -> Result<Kbd> {
    let dir = led(fs)?;
    Ok(Kbd {
        name: dir.rsplit('/').next().unwrap_or_default().to_string(),
        level: fs.parse(&format!("{dir}/brightness"))?,
        max: fs.parse(&format!("{dir}/max_brightness"))?,
    })
}

pub fn set(fs: &Sysfs, level: u32) -> Result<()> {
    let dir = led(fs)?;
    let max: u32 = fs.parse(&format!("{dir}/max_brightness"))?;
    fs.write(&format!("{dir}/brightness"), &level.min(max).to_string())
}

/// Next level, wrapping from the brightest back to off.
pub fn cycle(fs: &Sysfs) -> Result<u32> {
    let k = read(fs)?;
    let next = if k.level >= k.max { 0 } else { k.level + 1 };
    set(fs, next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::fake;

    #[test]
    fn cycles_through_levels() {
        let fs = fake(&[
            ("sys/class/leds/input3::capslock/brightness", "0"),
            ("sys/class/leds/tpacpi::kbd_backlight/brightness", "1"),
            ("sys/class/leds/tpacpi::kbd_backlight/max_brightness", "2"),
        ]);
        assert_eq!(read(&fs).unwrap().name, "tpacpi::kbd_backlight");
        assert_eq!(cycle(&fs).unwrap(), 2);
        assert_eq!(cycle(&fs).unwrap(), 0);
        set(&fs, 9).unwrap();
        assert_eq!(read(&fs).unwrap().level, 2);
    }
}
