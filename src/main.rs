mod bar;
mod tui;

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use fierro::config::Config;
use fierro::hw::battery::Battery;
use fierro::hw::charge::{self, Preset, Thresholds};
use fierro::hw::profile::{self, Profile, ProfileState};
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
