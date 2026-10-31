//! Fn-lock: on, F1–F12 work without Fn; off, the media keys do.

use super::{ideapad_dir, Error, Result, Sysfs, WMI_DIR};

/// The fn-lock attribute and the driver that owns it.
pub fn control(fs: &Sysfs) -> Result<(String, &'static str)> {
    let huawei = format!("{WMI_DIR}/fn_lock_state");
    if fs.exists(&huawei) {
        return Ok((huawei, "huawei-wmi"));
    }
    if let Some(dev) = ideapad_dir(fs) {
        let path = format!("{dev}/fn_lock");
        if fs.exists(&path) {
            return Ok((path, "ideapad_laptop"));
        }
    }
    Err(Error::Unsupported("fn-lock"))
}

pub fn read(fs: &Sysfs) -> Result<bool> {
    let (path, _) = control(fs)?;
    Ok(fs.parse::<u8>(&path)? != 0)
}

pub fn set(fs: &Sysfs, on: bool) -> Result<()> {
    let (path, _) = control(fs)?;
    fs.write(&path, if on { "1" } else { "0" })
}

pub fn toggle(fs: &Sysfs) -> Result<bool> {
    let on = !read(fs)?;
    set(fs, on)?;
    Ok(on)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::fake;

    #[test]
    fn finds_huawei_then_ideapad() {
        let fs = fake(&[("sys/devices/platform/huawei-wmi/fn_lock_state", "0")]);
        assert_eq!(control(&fs).unwrap().1, "huawei-wmi");
        let fs = fake(&[("sys/bus/platform/drivers/ideapad_acpi/VPC2004:00/fn_lock", "1")]);
        assert!(read(&fs).unwrap());
        assert!(!toggle(&fs).unwrap());
        assert!(matches!(read(&fake(&[])), Err(Error::Unsupported(_))));
    }
}
