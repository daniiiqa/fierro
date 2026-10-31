//! discada's shared TUI style (the `[style]` of each look, written to rola/look.json)
//! mapped onto ratatui. Colors are the terminal's own palette, which wallust sets.

use std::fs;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders};
use serde::Deserialize;

use fierro::config::config_dir;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Look {
    pub borders: String,
    pub bar: String,
    pub labels: String,
    pub case: String,
    pub select: String,
    pub tint: String,
    pub transparent: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            borders: "auto".into(),
            bar: "line".into(),
            labels: "icons".into(),
            case: "normal".into(),
            select: "fill".into(),
            tint: "color".into(),
            transparent: true,
        }
    }
}

impl Look {
    /// fierro's own look.json, else discada's shared one, else defaults.
    pub fn load() -> Self {
        let dir = config_dir();
        ["fierro/look.json", "rola/look.json"]
            .iter()
            .find_map(|rel| fs::read_to_string(dir.join(rel)).ok().and_then(|s| serde_json::from_str(&s).ok()))
            .unwrap_or_default()
    }
}

/// Extra named colors from discada's palette file (hex), when present.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Palette {
    background: Option<String>,
    critical: Option<String>,
    alert: Option<String>,
}

fn hex(s: &Option<String>) -> Option<Color> {
    let h = s.as_deref()?.strip_prefix('#')?;
    let v = u32::from_str_radix(h, 16).ok().filter(|_| h.len() == 6)?;
    Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub struct Theme {
    pub look: Look,
    pub fg: Style,
    pub dim: Style,
    pub acc: Style,
    pub acc2: Style,
    pub warn: Style,
    pub crit: Style,
    bg: Color,
}

impl Theme {
    pub fn new(look: Look) -> Self {
        let palette: Palette = fs::read_to_string(config_dir().join("empalme/colors.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let bold = Modifier::BOLD;
        let (fg, dim, acc, acc2) = match look.tint.as_str() {
            "phosphor" => (
                Style::new().fg(Color::Green),
                Style::new().fg(Color::Green).add_modifier(Modifier::DIM),
                Style::new().fg(Color::LightGreen).add_modifier(bold),
                Style::new().fg(Color::LightGreen),
            ),
            "ink" => (
                Style::new(),
                Style::new().add_modifier(Modifier::DIM),
                Style::new().add_modifier(bold),
                Style::new().add_modifier(Modifier::ITALIC),
            ),
            _ => (
                Style::new(),
                Style::new().fg(Color::Indexed(8)),
                Style::new().fg(Color::Indexed(12)).add_modifier(bold),
                Style::new().fg(Color::Indexed(13)),
            ),
        };
        Self {
            fg,
            dim,
            acc,
            acc2,
            warn: Style::new().fg(hex(&palette.alert).unwrap_or(Color::Yellow)),
            crit: Style::new().fg(hex(&palette.critical).unwrap_or(Color::Red)).add_modifier(bold),
            bg: hex(&palette.background).unwrap_or(Color::Reset),
            look,
        }
    }

    pub fn background(&self) -> Option<Style> {
        (!self.look.transparent).then(|| Style::new().bg(self.bg))
    }

    pub fn case(&self, s: &str) -> String {
        match self.look.case.as_str() {
            "upper" => s.to_uppercase(),
            "lower" => s.to_lowercase(),
            _ => s.to_string(),
        }
    }

    pub fn label(&self, icon: &str, short: &str, word: &str) -> String {
        self.case(&match self.look.labels.as_str() {
            "short" => short.to_string(),
            "words" => word.to_string(),
            _ => format!("{icon} {word}"),
        })
    }

    pub fn block(&self, title: String, focused: bool) -> Block<'static> {
        let border_type = match self.look.borders.as_str() {
            "plain" => BorderType::Plain,
            "double" => BorderType::Double,
            "thick" => BorderType::Thick,
            "dashed" => BorderType::LightDoubleDashed,
            _ => BorderType::Rounded,
        };
        let borders = if self.look.borders == "none" { Borders::NONE } else { Borders::ALL };
        let edge = if focused { self.acc } else { self.dim };
        Block::new()
            .borders(borders)
            .border_type(border_type)
            .border_style(edge.remove_modifier(Modifier::BOLD))
            .title(Span::styled(format!(" {title} "), if focused { self.acc } else { self.fg }))
    }

    /// (filled, empty, knob) glyphs for the `bar` style.
    fn glyphs(&self) -> (&'static str, &'static str, &'static str) {
        match self.look.bar.as_str() {
            "knob" => ("━", "─", "●"),
            "blocks" => ("█", "░", "▓"),
            "dots" => ("•", "·", "●"),
            "ascii" => ("#", "-", "|"),
            _ => ("━", "─", "━"),
        }
    }

    pub fn bar(&self, ratio: f64, width: u16, style: Style) -> Vec<Span<'static>> {
        let (fill, empty, knob) = self.glyphs();
        let w = width as usize;
        let n = ((ratio.clamp(0.0, 1.0) * w as f64).round() as usize).min(w);
        let mut spans = Vec::new();
        if self.look.bar == "ascii" {
            spans.push(Span::styled("[", self.dim));
        }
        if n > 0 {
            spans.push(Span::styled(fill.repeat(n - 1), style));
            spans.push(Span::styled(if n < w { knob } else { fill }, style));
        }
        spans.push(Span::styled(empty.repeat(w - n), self.dim));
        if self.look.bar == "ascii" {
            spans.push(Span::styled("]", self.dim));
        }
        spans
    }

    /// A 0–100 track with handles at `start` and `end`; `active` (0 or 1) is the one being edited.
    pub fn slider(&self, start: u8, end: u8, width: u16, active: Option<usize>) -> Vec<Span<'static>> {
        let (fill, empty, _) = self.glyphs();
        let handle = if self.look.bar == "ascii" { "|" } else { "●" };
        let w = width.max(2) as usize;
        let pos = |v: u8| (v as usize * (w - 1) + 50) / 100;
        let (ps, pe) = (pos(start), pos(end));
        (0..w)
            .map(|i| {
                let which = if i == ps { Some(0) } else if i == pe { Some(1) } else { None };
                match which {
                    Some(h) if active == Some(h) => Span::styled(handle, self.acc2.add_modifier(Modifier::BOLD)),
                    Some(_) => Span::styled(handle, self.acc),
                    None if i > ps && i < pe => Span::styled(fill, self.acc.remove_modifier(Modifier::BOLD)),
                    None => Span::styled(empty, self.dim),
                }
            })
            .collect()
    }

    /// One choice in a row of options: `current` is the hardware's state, `focused` the cursor row.
    pub fn choice(&self, text: &str, current: bool, focused: bool) -> Vec<Span<'static>> {
        let text = self.case(text);
        if !current {
            return vec![Span::styled(format!(" {text} "), self.dim)];
        }
        if !focused {
            return vec![Span::styled(format!(" {text} "), self.acc)];
        }
        match self.look.select.as_str() {
            "bar" => vec![Span::styled("▌", self.acc2), Span::styled(format!("{text} "), self.acc)],
            "inverse" => vec![Span::styled(format!(" {text} "), self.acc.add_modifier(Modifier::REVERSED))],
            _ => {
                let fill = self.acc.fg.unwrap_or(Color::Reset);
                if fill == Color::Reset {
                    vec![Span::styled(format!(" {text} "), self.acc.add_modifier(Modifier::REVERSED))]
                } else {
                    vec![Span::styled(format!(" {text} "), Style::new().bg(fill).fg(self.bg).add_modifier(Modifier::BOLD))]
                }
            }
        }
    }
}
