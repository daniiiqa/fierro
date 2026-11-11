//! Fingerprint readers (`fierro huella`): what the hardware is, who is enrolled,
//! and which PAM services take a finger.
//!
//! Detection and PAM go through [`Sysfs`] (so tests use fake trees, `etc/pam.d`
//! included); enrolling and deleting go through fprintd's own CLIs.

use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use serde::Serialize;

use super::{Error, Result, Sysfs};

/// Readers with no stock kernel driver, keyed by ACPI device, with where to get one.
const ACPI_READERS: [(&str, &str, &str); 1] =
    [("GXFP5130:00", "Goodix GXFP5130 (eSPI)", "https://github.com/Metrohan/gxfp5130-linux")];

/// USB vendors that make nothing but fingerprint readers.
const USB_VENDORS: [(&str, &str); 7] = [
    ("27c6", "Goodix"),
    ("138a", "Validity"),
    ("10a5", "FPC"),
    ("1c7a", "Egis"),
    ("298d", "Next Biometrics"),
    ("147e", "UPEK"),
    ("2808", "FocalTech"),
];

/// Synaptics and Elan also make touchpads and touchscreens: a reader only when the product says so.
const USB_MIXED: [(&str, &str); 2] = [("06cb", "Synaptics"), ("04f3", "Elan")];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reader {
    pub name: String,
    pub bus: &'static str,
    /// Where to get the kernel driver, when the reader needs one and it isn't bound.
    pub missing_driver: Option<&'static str>,
}

pub fn detect(fs: &Sysfs) -> Option<Reader> {
    for (id, name, url) in ACPI_READERS {
        let dev = format!("sys/bus/acpi/devices/{id}");
        if fs.exists(&dev) {
            let bound = fs.exists(&format!("{dev}/physical_node/driver"));
            return Some(Reader { name: name.into(), bus: "espi", missing_driver: (!bound).then_some(url) });
        }
    }
    const USB: &str = "sys/bus/usb/devices";
    fs.list(USB, "").into_iter().find_map(|d| {
        let vendor = fs.read_opt(&format!("{USB}/{d}/idVendor"))?;
        let product = fs.read_opt(&format!("{USB}/{d}/product")).unwrap_or_default();
        let maker = USB_VENDORS
            .iter()
            .find(|(v, _)| *v == vendor)
            .or_else(|| {
                let p = product.to_lowercase();
                USB_MIXED.iter().find(|(v, _)| *v == vendor && (p.contains("finger") || p.contains("fp")))
            })?
            .1;
        let name = if product.is_empty() { format!("{maker} reader") } else { format!("{maker} {product}") };
        Some(Reader { name, bus: "usb", missing_driver: None })
    })
}

/// What fprintd sees: its device, and the fingers enrolled for a user.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Enrolled {
    pub device: Option<String>,
    pub fingers: Vec<String>,
}

/// `fprintd-list <user>`; `None` when fprintd isn't installed.
pub fn enrolled(user: &str) -> Option<Enrolled> {
    let out = Command::new("fprintd-list").arg(user).stderr(Stdio::null()).output().ok()?;
    Some(parse_list(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_list(out: &str) -> Enrolled {
    let mut e = Enrolled::default();
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("Fingerprints for user ") {
            // "dany on Goodix GXFP5130 eSPI Fingerprint Sensor (press):"
            e.device = rest.split_once(" on ").map(|(_, d)| d.trim_end_matches(':').to_string());
        } else if let Some((_, finger)) = line.trim().strip_prefix("- #").and_then(|l| l.split_once(": ")) {
            e.fingers.push(finger.to_string());
        }
    }
    e
}

/// fprintd's name for a finger, from its own name or a short one in Spanish or
/// English: `indice`, `pulgar-izquierdo`, `left-thumb`, `ring-left`… Right hand
/// unless told otherwise.
pub fn finger_name(s: &str) -> Result<String> {
    let s = s.to_lowercase().replace(['á', 'í'], "i").replace('ñ', "n").replace([' ', '_'], "-");
    let mut side = "right";
    let mut finger = None;
    for word in s.split('-') {
        match word {
            "right" | "derecho" | "derecha" | "der" => side = "right",
            "left" | "izquierdo" | "izquierda" | "izq" => side = "left",
            "thumb" | "pulgar" => finger = Some("thumb"),
            "index" | "indice" => finger = Some("index"),
            "middle" | "medio" | "corazon" => finger = Some("middle"),
            "ring" | "anular" => finger = Some("ring"),
            "little" | "pinky" | "menique" => finger = Some("little"),
            "finger" | "dedo" | "" => {}
            _ => return Err(Error::Invalid(format!("not a finger: {s:?}"))),
        }
    }
    match finger.ok_or_else(|| Error::Invalid(format!("not a finger: {s:?}")))? {
        "thumb" => Ok(format!("{side}-thumb")),
        f => Ok(format!("{side}-{f}-finger")),
    }
}

/// One step of `fprintd-enroll`.
#[derive(Debug, Clone, PartialEq)]
pub enum EnrollStep {
    Passed,
    /// Touch again; why, in fprintd's words (`enroll-finger-not-centered`…).
    Retry(String),
    Done,
    Failed(String),
}

fn parse_enroll(line: &str) -> Option<EnrollStep> {
    let result = line.trim().strip_prefix("Enroll result: ")?;
    Some(match result {
        "enroll-stage-passed" => EnrollStep::Passed,
        "enroll-completed" => EnrollStep::Done,
        r if r.contains("retry") || r.contains("too-short") || r.contains("not-centered") => {
            EnrollStep::Retry(r.to_string())
        }
        r => EnrollStep::Failed(r.to_string()),
    })
}

/// Enrolls a finger (fprintd picks one when `None`), reporting each step.
pub fn enroll(finger: Option<&str>, mut step: impl FnMut(&EnrollStep)) -> Result<()> {
    let mut cmd = Command::new("fprintd-enroll");
    if let Some(f) = finger {
        cmd.args(["-f", f]);
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|source| Error::Io { path: "fprintd-enroll".into(), source })?;
    let mut last = None;
    for line in BufReader::new(child.stdout.take().expect("stdout is piped")).lines().map_while(|l| l.ok()) {
        if let Some(s) = parse_enroll(&line) {
            step(&s);
            last = Some(s);
        }
    }
    let _ = child.wait();
    match last {
        Some(EnrollStep::Done) => Ok(()),
        Some(EnrollStep::Failed(why)) => Err(Error::Invalid(why)),
        _ => Err(Error::Invalid("fprintd-enroll stopped before finishing".into())),
    }
}

/// Deletes one finger, or every finger of `user` when `None`.
pub fn delete(user: &str, finger: Option<&str>) -> Result<()> {
    let mut cmd = Command::new("fprintd-delete");
    cmd.arg(user);
    if let Some(f) = finger {
        cmd.args(["-f", f]);
    }
    let status = cmd.status().map_err(|source| Error::Io { path: "fprintd-delete".into(), source })?;
    if status.success() { Ok(()) } else { Err(Error::Invalid("fprintd-delete failed".into())) }
}

// ---- PAM -------------------------------------------------------------------

const PAM_DIR: &str = "etc/pam.d";
const POLKIT_VENDOR: &str = "usr/lib/pam.d/polkit-1";
/// Every line fierro adds ends with this, so `pam off` takes out exactly those.
const MARK: &str = "# fierro huella";
const POLKIT_HEADER: &str = "# fierro huella: copy of /usr/lib/pam.d/polkit-1";

/// The services fierro manages. sudo and polkit ask for the finger first and
/// fall back to the password after 10 s; SDDM takes the password as always,
/// and a finger when Enter is pressed with the field empty.
pub const SERVICES: [&str; 3] = ["sudo", "polkit-1", "sddm"];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PamService {
    pub name: &'static str,
    pub on: bool,
}

fn active(text: &str) -> bool {
    text.lines().any(|l| !l.trim_start().starts_with('#') && l.contains("pam_fprintd.so"))
}

fn service_text(fs: &Sysfs, name: &str) -> Option<String> {
    fs.read_opt(&format!("{PAM_DIR}/{name}"))
        .or_else(|| (name == "polkit-1").then(|| fs.read_opt(POLKIT_VENDOR)).flatten())
}

/// The managed services that exist here (no SDDM on a machine without it).
pub fn pam(fs: &Sysfs) -> Vec<PamService> {
    SERVICES
        .iter()
        .filter_map(|&name| service_text(fs, name).map(|t| PamService { name, on: active(&t) }))
        .collect()
}

fn without_ours(text: &str) -> Vec<&str> {
    text.lines().filter(|l| !l.ends_with(MARK) && *l != POLKIT_HEADER).collect()
}

fn join(lines: &[impl AsRef<str>]) -> String {
    lines.iter().map(AsRef::as_ref).collect::<Vec<_>>().join("\n") + "\n"
}

/// Finger first: a `sufficient pam_fprintd` before the first auth line.
fn finger_first(text: &str) -> Result<String> {
    let lines = without_ours(text);
    if active(&join(&lines)) {
        return Err(Error::Invalid("pam_fprintd is already there, not added by fierro".into()));
    }
    let at = lines
        .iter()
        .position(|l| l.trim_start().trim_start_matches('-').starts_with("auth"))
        .ok_or_else(|| Error::Invalid("no auth lines".into()))?;
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    out.insert(at, format!("auth       sufficient   pam_fprintd.so  timeout=10  {MARK}"));
    Ok(join(&out))
}

/// SDDM's auth stack, password first: Arch's system-login + system-auth with
/// pam_fprintd as the fallback when pam_unix fails (an empty password). Written
/// out instead of included so a finger still resets pam_faillock.
const SDDM_AUTH: [&str; 9] = [
    "auth       required                    pam_shells.so",
    "auth       requisite                   pam_nologin.so",
    "auth       required                    pam_faillock.so      preauth",
    "auth       [success=2 default=ignore]  pam_unix.so          try_first_pass",
    "auth       [success=1 default=bad]     pam_fprintd.so       timeout=15",
    "auth       [default=die]               pam_faillock.so      authfail",
    "auth       optional                    pam_permit.so",
    "auth       required                    pam_env.so",
    "auth       required                    pam_faillock.so      authsucc",
];
const SDDM_INCLUDE: &str = "auth        include     system-login";

fn is_sddm_include(l: &str) -> bool {
    l.split_whitespace().eq(["auth", "include", "system-login"])
}

fn sddm_on(text: &str) -> Result<String> {
    let lines = without_ours(text);
    let at = lines.iter().position(|l| is_sddm_include(l)).ok_or_else(|| {
        Error::Invalid("/etc/pam.d/sddm doesn't have Arch's `auth include system-login`; not touching it".into())
    })?;
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    out.splice(at..=at, SDDM_AUTH.iter().map(|l| format!("{l}  {MARK}")));
    Ok(join(&out))
}

fn sddm_off(text: &str) -> String {
    let mut out = Vec::new();
    let mut put = false;
    for l in text.lines() {
        if l.ends_with(MARK) {
            if !put {
                out.push(SDDM_INCLUDE);
                put = true;
            }
        } else {
            out.push(l);
        }
    }
    join(&out)
}

/// The new text of a service (`None`: delete the file, it was fierro's copy).
fn transform(name: &str, text: &str, on: bool) -> Result<Option<String>> {
    Ok(match (name, on) {
        ("sddm", true) => Some(sddm_on(text)?),
        ("sddm", false) => Some(sddm_off(text)),
        ("polkit-1", false) if text.starts_with(POLKIT_HEADER) => None,
        (_, true) => Some(finger_first(text)?),
        (_, false) => Some(join(&without_ours(text))),
    })
}

/// Turns the finger on or off in every managed service that exists. Needs root.
/// Returns what changed, one line per service.
pub fn set_pam(fs: &Sysfs, on: bool) -> Result<Vec<String>> {
    let mut done = Vec::new();
    for name in SERVICES {
        let path = fs.path(&format!("{PAM_DIR}/{name}"));
        let Some(text) = service_text(fs, name) else { continue };
        let own = fs.exists(&format!("{PAM_DIR}/{name}"));
        // Already as asked: leave it (and a pam_fprintd someone else put there) alone.
        if active(&text) == on {
            continue;
        }
        let io = |source| Error::Io { path: path.clone(), source };
        let new = transform(name, &text, on)?;
        if !on && new.as_deref().is_some_and(active) {
            return Err(Error::Invalid(format!("pam_fprintd in /etc/pam.d/{name} wasn't added by fierro; edit it by hand")));
        }
        // polkit lives in /usr/lib/pam.d; fierro's copy in /etc says whose it is.
        let new = new.map(|n| if own { n } else { format!("{POLKIT_HEADER}\n{n}") });
        match new {
            Some(new) => {
                if own {
                    let backup = path.with_extension("fierro-orig");
                    if !backup.exists() {
                        fs::copy(&path, &backup).map_err(io)?;
                    }
                }
                fs::write(&path, new).map_err(|source| match source.kind() {
                    std::io::ErrorKind::PermissionDenied => Error::Permission(path.clone()),
                    _ => io(source),
                })?;
            }
            None => fs::remove_file(&path).map_err(io)?,
        }
        done.push(name.to_string());
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::fake;

    const ARCH_SUDO: &str = "#%PAM-1.0\nauth\t\tinclude\t\tsystem-auth\naccount\t\tinclude\t\tsystem-auth\nsession\t\tinclude\t\tsystem-auth\n";
    const ARCH_SDDM: &str = "#%PAM-1.0\n\nauth        include     system-login\n-auth       optional    pam_gnome_keyring.so\n\naccount     include     system-login\n";

    #[test]
    fn finds_the_espi_goodix_and_its_driver() {
        let bare = fake(&[("sys/bus/acpi/devices/GXFP5130:00/hid", "GXFP5130")]);
        let r = detect(&bare).unwrap();
        assert_eq!(r.bus, "espi");
        assert!(r.missing_driver.is_some());
        let bound = fake(&[("sys/bus/acpi/devices/GXFP5130:00/physical_node/driver/x", "")]);
        assert_eq!(detect(&bound).unwrap().missing_driver, None);
    }

    #[test]
    fn usb_readers_but_not_touchpads() {
        let goodix = fake(&[("sys/bus/usb/devices/3-4/idVendor", "27c6")]);
        assert_eq!(detect(&goodix).unwrap().name, "Goodix reader");
        let touchpad = fake(&[("sys/bus/usb/devices/1-2/idVendor", "04f3"), ("sys/bus/usb/devices/1-2/product", "Touchscreen")]);
        assert_eq!(detect(&touchpad), None);
        let elan = fake(&[("sys/bus/usb/devices/1-2/idVendor", "04f3"), ("sys/bus/usb/devices/1-2/product", "ELAN:Fingerprint")]);
        assert!(detect(&elan).is_some());
    }

    #[test]
    fn parses_fprintd_list() {
        let out = "Found 1 devices\nDevice at /net/reactivated/Fprint/Device/0\nUsing device /net/reactivated/Fprint/Device/0\nFingerprints for user dany on Goodix GXFP5130 eSPI Fingerprint Sensor (press):\n - #0: right-index-finger\n - #1: left-thumb\n";
        let e = parse_list(out);
        assert_eq!(e.device.as_deref(), Some("Goodix GXFP5130 eSPI Fingerprint Sensor (press)"));
        assert_eq!(e.fingers, ["right-index-finger", "left-thumb"]);
        assert_eq!(parse_list("No devices available\n"), Enrolled::default());
    }

    #[test]
    fn finger_names_in_both_languages() {
        assert_eq!(finger_name("indice").unwrap(), "right-index-finger");
        assert_eq!(finger_name("pulgar-izquierdo").unwrap(), "left-thumb");
        assert_eq!(finger_name("Meñique izquierdo").unwrap(), "left-little-finger");
        assert_eq!(finger_name("left-ring-finger").unwrap(), "left-ring-finger");
        assert!(finger_name("nariz").is_err());
    }

    #[test]
    fn enroll_steps() {
        assert_eq!(parse_enroll("Enroll result: enroll-stage-passed"), Some(EnrollStep::Passed));
        assert_eq!(parse_enroll("Enroll result: enroll-completed"), Some(EnrollStep::Done));
        assert!(matches!(parse_enroll("Enroll result: enroll-finger-not-centered"), Some(EnrollStep::Retry(_))));
        assert!(matches!(parse_enroll("Enroll result: enroll-unknown-error"), Some(EnrollStep::Failed(_))));
        assert_eq!(parse_enroll("Enrolling right-index-finger finger."), None);
    }

    #[test]
    fn sudo_round_trip() {
        let on = transform("sudo", ARCH_SUDO, true).unwrap().unwrap();
        assert!(active(&on));
        assert!(on.lines().nth(1).unwrap().contains("pam_fprintd.so"), "finger goes before the first auth line");
        assert_eq!(transform("sudo", &on, false).unwrap().unwrap(), ARCH_SUDO);
    }

    #[test]
    fn sddm_round_trip() {
        let on = transform("sddm", ARCH_SDDM, true).unwrap().unwrap();
        assert!(active(&on) && !on.lines().any(is_sddm_include));
        let unix = on.lines().position(|l| l.contains("pam_unix")).unwrap();
        let finger = on.lines().position(|l| l.contains("pam_fprintd")).unwrap();
        assert!(unix < finger, "password first");
        let off = transform("sddm", &on, false).unwrap().unwrap();
        assert!(!active(&off) && off.lines().any(is_sddm_include));
        assert!(transform("sddm", "auth include common-auth\n", true).is_err());
    }

    #[test]
    fn set_pam_writes_backs_up_and_cleans_up() {
        let fs = fake(&[
            ("etc/pam.d/sudo", ARCH_SUDO),
            ("etc/pam.d/sddm", ARCH_SDDM),
            ("usr/lib/pam.d/polkit-1", "#%PAM-1.0\nauth include system-auth\n"),
        ]);
        assert_eq!(set_pam(&fs, true).unwrap(), ["sudo", "polkit-1", "sddm"]);
        assert!(pam(&fs).iter().all(|s| s.on));
        assert!(fs.exists("etc/pam.d/sudo.fierro-orig"));
        assert!(set_pam(&fs, true).unwrap().is_empty(), "idempotent");
        set_pam(&fs, false).unwrap();
        assert!(pam(&fs).iter().all(|s| !s.on));
        assert!(!fs.exists("etc/pam.d/polkit-1"), "fierro's polkit copy goes away");
        assert_eq!(fs.read("etc/pam.d/sudo").unwrap() + "\n", ARCH_SUDO);
    }
}
