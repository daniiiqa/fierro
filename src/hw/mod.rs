//! Hardware layer: everything fierro knows about the machine goes through sysfs.

pub mod battery;
pub mod charge;
pub mod fnlock;
pub mod led;
pub mod profile;
pub mod thermal;

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::str::FromStr;

use serde::Serialize;

pub(crate) const WMI_DIR: &str = "sys/devices/platform/huawei-wmi";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} is not available on this machine")]
    Unsupported(&'static str),
    #[error("permission denied writing {} (install dist/fierro-perms or run as root)", .0.display())]
    Permission(PathBuf),
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("unexpected value in {}: {value:?}", path.display())]
    Parse { path: PathBuf, value: String },
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Root of the sysfs tree. Real hardware lives at `/`; tests and `FIERRO_SYSFS_ROOT` point it at a fake tree.
#[derive(Debug, Clone)]
pub struct Sysfs {
    root: PathBuf,
}

impl Sysfs {
    pub fn new() -> Self {
        let root = std::env::var_os("FIERRO_SYSFS_ROOT").map_or_else(|| PathBuf::from("/"), PathBuf::from);
        Self { root }
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel.trim_start_matches('/'))
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    pub fn require(&self, rel: &str, feature: &'static str) -> Result<()> {
        if self.exists(rel) { Ok(()) } else { Err(Error::Unsupported(feature)) }
    }

    pub fn read(&self, rel: &str) -> Result<String> {
        let path = self.path(rel);
        fs::read_to_string(&path)
            .map(|s| s.trim().to_string())
            .map_err(|source| Error::Io { path, source })
    }

    pub fn read_opt(&self, rel: &str) -> Option<String> {
        self.read(rel).ok()
    }

    pub fn parse<T: FromStr>(&self, rel: &str) -> Result<T> {
        let value = self.read(rel)?;
        value.parse().map_err(|_| Error::Parse { path: self.path(rel), value })
    }

    pub fn parse_opt<T: FromStr>(&self, rel: &str) -> Option<T> {
        self.parse(rel).ok()
    }

    /// Writes an existing attribute; never creates files.
    pub fn write(&self, rel: &str, value: &str) -> Result<()> {
        let path = self.path(rel);
        OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .and_then(|mut f| f.write_all(value.as_bytes()))
            .map_err(|source| match source.kind() {
                io::ErrorKind::PermissionDenied => Error::Permission(path),
                _ => Error::Io { path, source },
            })
    }

    /// Entries of `rel` starting with `prefix`, in natural order (hwmon2 before hwmon10).
    pub fn list(&self, rel: &str, prefix: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.path(rel))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with(prefix))
            .collect();
        names.sort_by(|a, b| natural_key(a).cmp(&natural_key(b)));
        names
    }
}

impl Default for Sysfs {
    fn default() -> Self {
        Self::new()
    }
}

fn natural_key(s: &str) -> (&str, u64, &str) {
    let start = s.find(|c: char| c.is_ascii_digit()).unwrap_or(s.len());
    let end = s[start..].find(|c: char| !c.is_ascii_digit()).map_or(s.len(), |i| start + i);
    (&s[..start], s[start..end].parse().unwrap_or(0), &s[end..])
}

#[derive(Debug, Clone, Serialize)]
pub struct Machine {
    pub vendor: String,
    pub product: String,
    pub version: String,
    pub bios: String,
    pub huawei_wmi: bool,
}

impl Machine {
    pub fn detect(fs: &Sysfs) -> Self {
        let dmi = |f: &str| fs.read_opt(&format!("sys/class/dmi/id/{f}")).unwrap_or_default();
        Self {
            vendor: dmi("sys_vendor"),
            product: dmi("product_name"),
            version: dmi("product_version"),
            bios: dmi("bios_version"),
            huawei_wmi: fs.exists(WMI_DIR),
        }
    }
}

#[cfg(test)]
pub(crate) fn fake(files: &[(&str, &str)]) -> Sysfs {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_in_natural_order() {
        let fs = fake(&[("d/hwmon10/x", ""), ("d/hwmon2/x", ""), ("d/hwmon1/x", ""), ("d/other/x", "")]);
        assert_eq!(fs.list("d", "hwmon"), ["hwmon1", "hwmon2", "hwmon10"]);
    }

    #[test]
    fn write_never_creates() {
        let fs = fake(&[("a", "1")]);
        assert!(matches!(fs.write("missing", "1"), Err(Error::Io { .. })));
        fs.write("a", "2").unwrap();
        assert_eq!(fs.read("a").unwrap(), "2");
    }
}
