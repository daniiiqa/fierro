use super::{Result, Sysfs};

const MICMUTE: &str = "sys/class/leds/platform::micmute/brightness";

pub fn micmute(fs: &Sysfs) -> Result<bool> {
    fs.require(MICMUTE, "mic-mute LED")?;
    Ok(fs.parse::<u32>(MICMUTE)? != 0)
}

pub fn set_micmute(fs: &Sysfs, on: bool) -> Result<()> {
    fs.require(MICMUTE, "mic-mute LED")?;
    fs.write(MICMUTE, if on { "1" } else { "0" })
}
