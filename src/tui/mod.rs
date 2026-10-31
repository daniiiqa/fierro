//! The dashboard: `fierro` with no arguments in a terminal.

mod theme;
mod ui;

use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant, SystemTime};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

use fierro::config::Config;
use fierro::hw::charge::{self, Kind, Preset, Thresholds};
use fierro::hw::profile::{self, Profile};
use fierro::hw::{fnlock, kbd, Result, Sysfs};
use fierro::status::Status;

use theme::{Look, Theme};

const HISTORY: usize = 240;
const STEP: i16 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Preset,
    Start,
    End,
    Profile,
    Fnlock,
    Kbd,
}

pub struct App {
    fs: Sysfs,
    config: Config,
    pub theme: Theme,
    pub status: Status,
    pub row: Row,
    /// Charge limits being edited, applied on enter.
    pub draft: Option<(u8, u8)>,
    /// Power draw samples in tenths of a watt, newest last.
    pub power: VecDeque<u64>,
    /// Last action's result: (text, is_error).
    pub message: Option<(String, bool)>,
    last_read: Instant,
    theme_stamp: Vec<Option<SystemTime>>,
    restyle_ticks: u8,
    restyled: bool,
    quit: bool,
}

pub fn run(fs: Sysfs) -> io::Result<()> {
    let mut app = App::new(fs);
    let mut terminal = ratatui::init();
    let result = app.event_loop(&mut terminal);
    ratatui::restore();
    result
}

impl App {
    fn new(fs: Sysfs) -> Self {
        let status = Status::read(&fs);
        let mut app = Self {
            config: Config::load(),
            theme: Theme::new(Look::load()),
            status,
            fs,
            row: Row::Preset,
            draft: None,
            power: VecDeque::with_capacity(HISTORY),
            message: None,
            last_read: Instant::now(),
            theme_stamp: theme::stamp(),
            restyle_ticks: 0,
            restyled: false,
            quit: false,
        };
        app.sample();
        app.row = app.rows().first().copied().unwrap_or(Row::Profile);
        app
    }

    /// The rows this machine has, top to bottom.
    pub fn rows(&self) -> Vec<Row> {
        let s = &self.status;
        let mut rows = Vec::new();
        if let Some(kind) = s.charge_kind {
            rows.push(Row::Preset);
            if kind == Kind::Range {
                rows.push(Row::Start);
            }
            if kind != Kind::Toggle {
                rows.push(Row::End);
            }
        }
        if s.profile.is_some() {
            rows.push(Row::Profile);
        }
        if s.fnlock.is_some() {
            rows.push(Row::Fnlock);
        }
        if s.kbd.is_some() {
            rows.push(Row::Kbd);
        }
        rows
    }

    fn move_row(&mut self, by: isize) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let i = rows.iter().position(|&r| r == self.row).unwrap_or(0) as isize;
        self.row = rows[(i + by).rem_euclid(rows.len() as isize) as usize];
    }

    pub fn t(&self, es: &'static str, en: &'static str) -> &'static str {
        if self.config.spanish() { es } else { en }
    }

    fn event_loop(&mut self, terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
        while !self.quit {
            if std::mem::take(&mut self.restyled) {
                // ratatui only redraws changed cells; a new theme changes all of them
                terminal.clear()?;
            }
            terminal.draw(|f| ui::draw(f, self))?;
            if event::poll(Duration::from_millis(250))?
                && let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
            {
                self.key(k.code);
            }
            if self.last_read.elapsed() >= Duration::from_secs(1) {
                self.refresh();
            }
        }
        Ok(())
    }

    fn refresh(&mut self) {
        self.status = Status::read(&self.fs);
        self.last_read = Instant::now();
        self.sample();
        self.follow_look();
    }

    /// Restyle when a look switch rewrites the style or palette files. Rebuilt on two
    /// ticks, in case the first read caught a file mid-write.
    fn follow_look(&mut self) {
        let stamp = theme::stamp();
        if stamp != self.theme_stamp {
            self.theme_stamp = stamp;
            self.restyle_ticks = 2;
        }
        if self.restyle_ticks > 0 {
            self.restyle_ticks -= 1;
            self.theme = Theme::new(Look::load());
            self.restyled = true;
        }
    }

    fn sample(&mut self) {
        if let Some(b) = &self.status.battery {
            if self.power.len() == HISTORY {
                self.power.pop_front();
            }
            self.power.push_back((b.power_w * 10.0).round() as u64);
        }
    }

    fn key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc if self.draft.is_some() => self.draft = None,
            KeyCode::Esc => self.quit = true,
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => self.move_row(-1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => self.move_row(1),
            KeyCode::Left | KeyCode::Char('h') => self.adjust(-1),
            KeyCode::Right | KeyCode::Char('l') => self.adjust(1),
            KeyCode::Enter | KeyCode::Char(' ') => self.confirm(),
            KeyCode::Char('r') => self.refresh(),
            _ => {}
        }
    }

    /// The limits on screen: the draft while editing, else the hardware's.
    pub fn limits(&self) -> (u8, u8) {
        self.draft
            .or(self.status.charge.map(|t| (t.start, t.end)))
            .unwrap_or((0, 100))
    }

    fn adjust(&mut self, dir: i16) {
        let step = |v: u8| (v as i16 + dir * STEP).clamp(0, 100) as u8;
        match self.row {
            Row::Preset => {
                let all = Preset::for_kind(self.status.charge_kind.unwrap_or(Kind::Range));
                let next = match self.status.preset.and_then(|p| all.iter().position(|&q| q == p)) {
                    Some(i) => all[(i as i16 + dir).rem_euclid(all.len() as i16) as usize],
                    None if dir > 0 => all[0],
                    None => all[all.len() - 1],
                };
                self.set_charge(next.thresholds());
            }
            Row::Start => {
                let (s, e) = self.limits();
                self.draft = Some((step(s).min(e - STEP as u8), e));
            }
            Row::End => {
                let (s, e) = self.limits();
                self.draft = Some((s, step(e).max(s + STEP as u8)));
            }
            Row::Profile => {
                let cur = self.status.profile.as_ref().and_then(|p| p.profile).unwrap_or(Profile::Balance);
                // three profiles: going back one is going forward two
                let next = if dir > 0 { cur.next() } else { cur.next().next() };
                self.set_profile(next);
            }
            Row::Fnlock => self.toggle_fnlock(),
            Row::Kbd => {
                if let Some(k) = &self.status.kbd {
                    let level = (k.level as i16 + dir).clamp(0, k.max as i16) as u32;
                    let r = kbd::set(&self.fs, level);
                    self.report(r, format!("{} {level}/{}", self.t("teclado", "keyboard"), k.max));
                }
            }
        }
    }

    fn confirm(&mut self) {
        match self.row {
            Row::Start | Row::End => {
                if let Some((s, e)) = self.draft.take() {
                    match Thresholds::new(s, e) {
                        Ok(t) => self.set_charge(t),
                        Err(e) => self.message = Some((e.to_string(), true)),
                    }
                }
            }
            Row::Fnlock => self.toggle_fnlock(),
            Row::Kbd => {
                let r = kbd::cycle(&self.fs);
                let text = r.as_ref().map(|l| format!("{} {l}", self.t("teclado", "keyboard"))).unwrap_or_default();
                self.report(r.map(|_| ()), text);
            }
            _ => {}
        }
    }

    fn set_charge(&mut self, t: Thresholds) {
        self.draft = None;
        let r = charge::write(&self.fs, t).and_then(|()| {
            self.config.charge = Some(t);
            self.config.save()
        });
        let text = charge::describe(self.status.charge_kind.unwrap_or(Kind::Range), t);
        self.report(r, format!("{} {text}", self.t("carga", "charge")));
    }

    fn set_profile(&mut self, p: Profile) {
        let r = profile::apply(&self.fs, p).and_then(|()| {
            self.config.profile = Some(p);
            self.config.save()
        });
        self.report(r, format!("{} {}", self.t("perfil", "profile"), p.name()));
    }

    fn toggle_fnlock(&mut self) {
        let r = fnlock::toggle(&self.fs);
        let text = match r {
            Ok(on) => format!("fn-lock {}", if on { "on" } else { "off" }),
            Err(_) => String::new(),
        };
        self.report(r.map(|_| ()), text);
    }

    fn report(&mut self, r: Result<()>, ok: String) {
        self.message = Some(match r {
            Ok(()) => (ok, false),
            Err(e) => (e.to_string(), true),
        });
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use fierro::testing::machines;

    /// The dashboard for a fake machine, in English with the default look, as text.
    fn render(files: &[(&str, &str)]) -> (App, String) {
        let mut app = App::new(fierro::testing::fake(files));
        app.config.lang = "en".into();
        app.theme = Theme::new(Look::default());
        let mut terminal = Terminal::new(TestBackend::new(110, 22)).unwrap();
        terminal.draw(|f| ui::draw(f, &app)).unwrap();
        let buf = terminal.backend().buffer();
        let text = (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        println!("{text}");
        (app, text)
    }

    #[test]
    fn thinkpad_dashboard() {
        let (app, text) = render(&machines::thinkpad());
        assert_eq!(app.rows(), [Row::Preset, Row::Start, Row::End, Row::Profile, Row::Kbd]);
        assert!(text.contains("ThinkPad X1 Carbon Gen 11"));
        assert!(text.contains("from") && text.contains("keyboard") && text.contains("1/2"));
        assert!(!text.contains("fn-lock"));
    }

    #[test]
    fn asus_dashboard_has_one_handle() {
        let (app, text) = render(&machines::asus());
        assert_eq!(app.rows(), [Row::Preset, Row::End, Row::Profile, Row::Kbd]);
        assert!(!text.contains("from") && text.contains(" to "));
        assert!(text.contains("quiet"));
    }

    #[test]
    fn ideapad_dashboard_is_a_switch() {
        let (mut app, text) = render(&machines::ideapad());
        assert_eq!(app.rows(), [Row::Preset, Row::Profile, Row::Fnlock]);
        assert!(text.contains("conservation mode") && !text.contains("oficina"));
        app.row = Row::Fnlock;
        app.move_row(1);
        assert_eq!(app.row, Row::Preset);
    }

    #[test]
    fn huawei_dashboard_has_no_keyboard_panel() {
        let (app, text) = render(&machines::huawei());
        assert_eq!(app.rows(), [Row::Preset, Row::Start, Row::End, Row::Profile, Row::Fnlock]);
        assert!(text.contains("fn-lock") && !text.contains("keyboard"));
    }

    /// Renders the live dashboard as text: `cargo test preview -- --ignored --nocapture`
    /// (FIERRO_LOOK=borders,bar,labels,case,select picks a style).
    #[test]
    #[ignore]
    fn preview() {
        let mut app = App::new(Sysfs::new());
        if let Ok(spec) = std::env::var("FIERRO_LOOK") {
            let v: Vec<String> = spec.split(',').map(String::from).collect();
            let look = Look {
                borders: v[0].clone(),
                bar: v[1].clone(),
                labels: v[2].clone(),
                case: v[3].clone(),
                select: v[4].clone(),
                ..Look::default()
            };
            app.theme = Theme::new(look);
        }
        app.row = Row::Start;
        app.draft = Some((45, 70));
        let mut terminal = Terminal::new(TestBackend::new(96, 20)).unwrap();
        terminal.draw(|f| ui::draw(f, &app)).unwrap();
        let buf = terminal.backend().buffer();
        for y in 0..buf.area.height {
            let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            println!("{}", row.trim_end());
        }
    }
}
