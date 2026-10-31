use super::{Result, Sysfs};

const FN_LOCK: &str = "sys/devices/platform/huawei-wmi/fn_lock_state";

pub fn read(fs: &Sysfs) -> Result<bool> {
    fs.require(FN_LOCK, "fn-lock")?;
    Ok(fs.parse::<u8>(FN_LOCK)? != 0)
}

pub fn set(fs: &Sysfs, on: bool) -> Result<()> {
    fs.require(FN_LOCK, "fn-lock")?;
    fs.write(FN_LOCK, if on { "1" } else { "0" })
}

pub fn toggle(fs: &Sysfs) -> Result<bool> {
    let on = !read(fs)?;
    set(fs, on)?;
    Ok(on)
}
