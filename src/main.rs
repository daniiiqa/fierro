mod bar;
mod tui;

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use fierro::config::Config;
use fierro::hw::battery::Battery;
use fierro::hw::charge::{self, Preset, Thresholds};
use fierro::hw::profile::{self, Profile, ProfileState};
use fierro::hw::fingerprint::{self, EnrollStep};
use fierro::hw::{fnlock, kbd, Error, Result, Sysfs};
use fierro::report;
use fierro::status::Status;

#[derive(Parser)]
#[command(name = "fierro", version, about = "Huawei MateBook hardware, a todo fierro")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open the dashboard (the default in a terminal)
    Tui,
    /// Show everything fierro can see
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Show or set charge limits: a preset (casa, oficina, viaje) or START END
    Charge {
        #[arg(num_args = 0..=2)]
        args: Vec<String>,
    },
    /// Show or set fn-lock (on: F1–F12, off: media keys)
    Fnlock { state: Option<Switch> },
    /// Show or set the power profile (ahorro, balance, fierro, next)
    Profile {
        name: Option<String>,
        /// Apply without remembering it for `fierro restore`
        #[arg(long)]
        temp: bool,
    },
    /// Show or set the keyboard backlight: a level, or `next`
    Kbd { level: Option<String> },
    /// One-line battery health report
    Health,
    /// One line of waybar custom-module JSON
    Bar,
    /// Reapply the saved charge limits and profile (run at login)
    Restore,
    /// What this laptop exposes, as JSON to paste into a bug report
    Report,
    /// Fingerprint reader: status, enroll and delete fingers, and the logins that take one
    #[command(alias = "fingerprint")]
    Huella {
        #[command(subcommand)]
        action: Option<Huella>,
    },
}

#[derive(Subcommand)]
enum Huella {
    /// Enroll a finger (fprintd picks one if none given); Spanish names work: indice, pulgar-izquierdo…
    #[command(alias = "registrar")]
    Enroll { finger: Option<String> },
    /// Delete a finger, or every one when none is given
    #[command(alias = "borrar")]
    Delete { finger: Option<String> },
    /// Finger for sudo, polkit and SDDM (runs sudo itself when needed)
    Pam { state: Switch },
}

#[derive(Clone, Copy, ValueEnum)]
enum Switch {
    On,
    Off,
    Toggle,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let fs = Sysfs::new();
    let cmd = cli.cmd.unwrap_or(if std::io::stdout().is_terminal() { Cmd::Tui } else { Cmd::Status { json: false } });
    match run(&fs, cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fierro: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(fs: &Sysfs, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Tui => tui::run(fs.clone()).map_err(|source| Error::Io { path: "terminal".into(), source })?,
        Cmd::Status { json: true } => {
            println!("{}", serde_json::to_string(&Status::read(fs)).expect("status serializes"));
        }
        Cmd::Status { json: false } => print_status(&Status::read(fs)),
        Cmd::Charge { args } => {
            let target = match args.as_slice() {
                [] => None,
                [preset] => Some(preset.parse::<Preset>()?.thresholds()),
                [start, end] => Some(Thresholds::new(percent(start)?, percent(end)?)?),
                _ => unreachable!("clap limits charge to two args"),
            };
            if let Some(t) = target {
                charge::write(fs, t)?;
                let mut config = Config::load();
                config.charge = Some(t);
                config.save()?;
            }
            let status = Status::read(fs);
            println!("charge  {}", status.describe_charge().ok_or(Error::Unsupported("charge limits"))?);
        }
        Cmd::Fnlock { state } => {
            let on = match state {
                None => fnlock::read(fs)?,
                Some(Switch::Toggle) => fnlock::toggle(fs)?,
                Some(s) => {
                    fnlock::set(fs, matches!(s, Switch::On))?;
                    fnlock::read(fs)?
                }
            };
            println!("fn-lock  {}", describe_fnlock(on));
        }
        Cmd::Profile { name, temp } => {
            if let Some(name) = name {
                let target = match name.as_str() {
                    "next" => profile::read(fs)?.profile.map_or(Profile::Balance, Profile::next),
                    other => other.parse()?,
                };
                profile::apply(fs, target)?;
                if !temp {
                    let mut config = Config::load();
                    config.profile = Some(target);
                    config.save()?;
                }
            }
            println!("profile  {}", describe_profile(&profile::read(fs)?));
        }
        Cmd::Kbd { level } => {
            match level.as_deref() {
                None => {}
                Some("next") => {
                    kbd::cycle(fs)?;
                }
                Some(n) => kbd::set(fs, n.parse().map_err(|_| Error::Invalid(format!("not a level: {n:?}")))?)?,
            }
            let k = kbd::read(fs)?;
            println!("kbd  {}/{}", k.level, k.max);
        }
        Cmd::Health => println!("{}", describe_health(&Battery::read(fs)?)),
        Cmd::Bar => println!("{}", bar::render(&Status::read(fs), Config::load().spanish())),
        Cmd::Restore => {
            for done in Config::load().restore(fs)? {
                println!("restored {done}");
            }
        }
        Cmd::Report => println!("{}", serde_json::to_string_pretty(&report::report(fs)).expect("report serializes")),
        Cmd::Huella { action } => huella(fs, action)?,
    }
    Ok(())
}

fn percent(s: &str) -> Result<u8> {
    s.trim_end_matches('%').parse().map_err(|_| Error::Invalid(format!("not a percentage: {s:?}")))
}

fn print_status(s: &Status) {
    let m = &s.machine;
    println!("{}", m.title());
    if m.vendor.eq_ignore_ascii_case("huawei") && !m.huawei_wmi {
        println!("warning  huawei-wmi not loaded — try `sudo modprobe huawei-wmi`");
    }
    if let Some(b) = &s.battery {
        let left = b.minutes_left.map(|m| format!(" · {}h {:02}m left", m / 60, m % 60)).unwrap_or_default();
        let state = if s.held { "held at limit".to_string() } else { b.status.to_lowercase() };
        println!("battery  {}% {state} · {:.1} W{left}", b.capacity, b.power_w);
        println!("health   {}", describe_health(b));
    }
    if let Some(charge) = s.describe_charge() {
        println!("charge   {charge}");
    }
    if let Some(on) = s.fnlock {
        println!("fn-lock  {}", describe_fnlock(on));
    }
    if let Some(p) = &s.profile {
        println!("profile  {}", describe_profile(p));
    }
    if let Some(k) = &s.kbd {
        println!("kbd      {}/{}", k.level, k.max);
    }
    if let Some(r) = &s.fingerprint {
        let driver = if r.missing_driver.is_some() { " · no driver (fierro huella)" } else { "" };
        println!("finger   {}{driver}", r.name);
    }
    let mut thermal = Vec::new();
    if let Some(c) = s.cpu_celsius {
        thermal.push(format!("cpu {c:.0}°C"));
    }
    thermal.extend(s.thermals.fans.iter().map(|f| format!("fan {} rpm", f.rpm)));
    if !thermal.is_empty() {
        println!("thermal  {}", thermal.join(" · "));
    }
}

fn describe_profile(p: &ProfileState) -> String {
    let name = p.profile.map_or("custom", Profile::name);
    let mut parts: Vec<&str> = [&p.platform, &p.governor, &p.epp].into_iter().flatten().map(String::as_str).collect();
    match p.turbo {
        Some(true) => parts.push("turbo on"),
        Some(false) => parts.push("turbo off"),
        None => {}
    }
    format!("{name} ({})", parts.join(" / "))
}

fn describe_health(b: &Battery) -> String {
    let cycles = b.cycles.map(|c| format!(" · {c} cycles")).unwrap_or_default();
    format!("{:.1}% ({:.1} / {:.1} Wh){cycles}", b.health_pct, b.energy_full_wh, b.energy_design_wh)
}

fn describe_fnlock(on: bool) -> &'static str {
    if on { "on (F1–F12)" } else { "off (media keys)" }
}

fn user() -> String {
    std::env::var("USER").unwrap_or_default()
}

fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("Uid:").map(|u| u.split_whitespace().next() == Some("0"))))
        .unwrap_or(false)
}

fn huella(fs: &Sysfs, action: Option<Huella>) -> Result<()> {
    let es = Config::load().spanish();
    let t = move |a: &'static str, b: &'static str| if es { a } else { b };
    let finger = |f: Option<String>| f.map(|f| fingerprint::finger_name(&f)).transpose();
    match action {
        None => {
            let reader = fingerprint::detect(fs);
            let enrolled = fingerprint::enrolled(&user()).unwrap_or_default();
            let name = enrolled.device.clone().or_else(|| reader.as_ref().map(|r| r.name.clone()));
            let Some(name) = name else { return Err(Error::Unsupported("a fingerprint reader")) };
            println!("{}  {name}", t("lector ", "reader "));
            if let Some(url) = reader.as_ref().and_then(|r| r.missing_driver) {
                println!("         {} {url}", t("le falta el driver del kernel:", "needs a kernel driver:"));
            } else if enrolled.device.is_none() {
                println!("         {}", t("fprintd no lo ve (¿falta fprintd o libfprint?)", "fprintd can't see it (fprintd or libfprint missing?)"));
            }
            let fingers = if enrolled.fingers.is_empty() {
                t("ninguno — fierro huella registrar", "none — fierro huella enroll").to_string()
            } else {
                enrolled.fingers.join(", ")
            };
            println!("{}  {fingers}", t("dedos  ", "fingers"));
            for s in fingerprint::pam(fs) {
                let mark = if s.on { t("✓ huella", "✓ finger") } else { t("· contraseña", "· password") };
                println!("{:<9}{mark}", s.name);
            }
        }
        Some(Huella::Enroll { finger: f }) => {
            let f = finger(f)?;
            println!("{}", t("Toca el lector; levanta el dedo y muévelo un poco entre toques.", "Touch the reader; lift and shift your finger a little between touches."));
            let mut n = 0;
            let res = fingerprint::enroll(f.as_deref(), |step| match step {
                EnrollStep::Passed => {
                    n += 1;
                    println!("  ✓ {n}");
                }
                EnrollStep::Retry(why) => {
                    let hint = if why.contains("centered") {
                        t("céntralo", "center it")
                    } else if why.contains("remove") {
                        t("quítalo y vuelve a ponerlo", "lift it and touch again")
                    } else {
                        t("otra vez", "again")
                    };
                    println!("  · {hint}");
                }
                EnrollStep::Done => println!("{}", t("Listo, registrado.", "Done, enrolled.")),
                EnrollStep::Failed(_) => {}
            });
            if let Err(e) = res {
                if fingerprint::detect(fs).is_some_and(|r| r.bus == "espi") {
                    eprintln!("{}", t(
                        "Si se tardó y falló: sudo systemctl stop fprintd; sudo modprobe -r gxfp; sudo modprobe gxfp — y otra vez.",
                        "If it timed out: sudo systemctl stop fprintd; sudo modprobe -r gxfp; sudo modprobe gxfp — then retry.",
                    ));
                }
                return Err(e);
            }
        }
        Some(Huella::Delete { finger: f }) => {
            let f = finger(f)?;
            fingerprint::delete(&user(), f.as_deref())?;
            println!("{}", t("Borrado.", "Deleted."));
        }
        Some(Huella::Pam { state }) => {
            let on = match state {
                Switch::On => true,
                Switch::Off => false,
                Switch::Toggle => !fingerprint::pam(fs).iter().any(|s| s.on),
            };
            if is_root() {
                for name in fingerprint::set_pam(fs, on)? {
                    println!("{name}");
                }
            } else {
                let exe = std::env::current_exe().map_err(|source| Error::Io { path: "fierro".into(), source })?;
                let ok = std::process::Command::new("sudo")
                    .arg(exe)
                    .args(["huella", "pam", if on { "on" } else { "off" }])
                    .stdout(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                if !ok {
                    return Err(Error::Invalid(t("no se pudo con sudo", "sudo failed").into()));
                }
                return huella(fs, None);
            }
        }
    }
    Ok(())
}
