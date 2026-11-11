use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Sparkline};
use ratatui::Frame;

use fierro::hw::charge::{Kind, Preset};
use fierro::hw::profile::Profile;

use super::{App, Row};

pub fn draw(f: &mut Frame, app: &App) {
    let th = &app.theme;
    if let Some(bg) = th.background() {
        f.render_widget(Block::new().style(bg), f.area());
    }
    let area = f.area().inner(Margin { horizontal: 1, vertical: 0 });
    let charge_rows = if app.status.charge_kind.is_some() { 5 } else { 0 };
    let [header, battery, charge, bottom, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(7),
        Constraint::Length(charge_rows),
        Constraint::Length(5),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, app, header);
    draw_battery(f, app, battery);
    if charge_rows > 0 {
        draw_charge(f, app, charge);
    }
    // Only the panels this machine has, sharing the row by weight.
    type Draw = fn(&mut Frame, &App, Rect);
    let mut panels: Vec<(u16, Draw)> = Vec::new();
    if app.status.profile.is_some() {
        panels.push((3, draw_profile));
    }
    if app.status.fnlock.is_some() {
        panels.push((2, draw_fnlock));
    }
    if app.status.kbd.is_some() {
        panels.push((2, draw_kbd));
    }
    if app.huella.is_some() {
        panels.push((2, draw_huella));
    }
    panels.push((3, draw_thermal));
    let areas = Layout::horizontal(panels.iter().map(|(w, _)| Constraint::Fill(*w))).split(bottom);
    for ((_, draw), area) in panels.iter().zip(areas.iter()) {
        draw(f, app, *area);
    }
    draw_footer(f, app, footer);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let m = &app.status.machine;
    let line = Line::from(vec![
        Span::styled(th.case(" fierro "), th.acc),
        Span::styled(format!(" {}", m.title()), th.dim),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_battery(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let block = th.block(th.label("󰁹", "bat", app.t("batería", "battery")), false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(b) = &app.status.battery else {
        f.render_widget(Paragraph::new(Span::styled(app.t("sin batería", "no battery"), th.dim)), inner);
        return;
    };
    let [charge_row, health_row, _, graph] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)])
            .areas(inner);
    let bar_w = inner.width.saturating_sub(44).clamp(10, 40);

    let level = if b.capacity <= 10 && !b.is_charging() {
        th.crit
    } else if b.capacity <= 20 && !b.is_charging() {
        th.warn
    } else {
        th.acc
    };
    let state = if app.status.held {
        app.t("detenida en el límite", "held at limit")
    } else {
        match b.status.as_str() {
            "Charging" => app.t("cargando", "charging"),
            "Discharging" => app.t("con batería", "on battery"),
            "Full" => app.t("llena", "full"),
            _ => app.t("conectada", "plugged in"),
        }
    };
    let left = b.minutes_left.map(|m| format!(" · {}h {:02}m", m / 60, m % 60)).unwrap_or_default();
    let mut line = vec![Span::styled(format!("{:<7}", th.case(app.t("carga", "charge"))), th.dim)];
    line.extend(th.bar(b.capacity as f64 / 100.0, bar_w, level));
    line.push(Span::styled(format!(" {:>3}%", b.capacity), level));
    line.push(Span::styled(format!("  {} · {:.1} W{left}", th.case(state), b.power_w), th.fg));
    f.render_widget(Paragraph::new(Line::from(line)), charge_row);

    let health = if b.health_pct < 70.0 { th.warn } else { th.acc2 };
    let cycles = b.cycles.map(|c| format!(" · {c} {}", app.t("ciclos", "cycles"))).unwrap_or_default();
    let mut line = vec![Span::styled(format!("{:<7}", th.case(app.t("salud", "health"))), th.dim)];
    line.extend(th.bar(b.health_pct / 100.0, bar_w, health));
    line.push(Span::styled(format!(" {:>3.0}%", b.health_pct), health));
    line.push(Span::styled(
        format!("  {:.1} / {:.1} Wh{cycles}", b.energy_full_wh, b.energy_design_wh),
        th.fg,
    ));
    f.render_widget(Paragraph::new(Line::from(line)), health_row);

    let [label, spark] = Layout::horizontal([Constraint::Length(7), Constraint::Min(1)]).areas(graph);
    let samples: Vec<u64> = app.power.iter().rev().take(spark.width as usize).rev().copied().collect();
    // Tenths of a watt; headroom and a 25 W floor keep a steady draw from filling the graph.
    let peak = samples.iter().copied().max().unwrap_or(0);
    let label_lines = vec![
        Line::from(Span::styled(th.case(app.t("consumo", "power")), th.dim)),
        Line::from(Span::styled(format!("{:.0} W", peak as f64 / 10.0), th.dim)),
        Line::from(Span::styled(th.case(app.t("máx", "peak")), th.dim)),
    ];
    f.render_widget(Paragraph::new(label_lines), label);
    f.render_widget(Sparkline::default().data(samples).max((peak * 5 / 4).max(250)).style(th.acc2), spark);
}

fn draw_charge(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let focused = matches!(app.row, Row::Preset | Row::Start | Row::End);
    let block = th.block(th.label("󰂄", app.t("límite", "limit"), app.t("carga inteligente", "smart charge")), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [presets, slider, values] = Layout::vertical([Constraint::Length(1); 3]).areas(inner);
    let kind = app.status.charge_kind.unwrap_or(Kind::Range);

    let current = if app.draft.is_some() { None } else { app.status.preset };
    let mut line = Vec::new();
    for &p in Preset::for_kind(kind) {
        line.extend(th.choice(p.name(), current == Some(p), app.row == Row::Preset));
    }
    if current.is_none() {
        line.extend(th.choice(app.t("a mano", "custom"), true, app.row == Row::Preset));
    }
    f.render_widget(Paragraph::new(Line::from(line)), presets);

    if kind == Kind::Toggle {
        let note = app.t(
            "modo conservación: el firmware la detiene entre 55 y 80%",
            "conservation mode: the firmware stops charging around 55–80%",
        );
        f.render_widget(Paragraph::new(Span::styled(th.case(&format!("  {note}")), th.dim)), slider);
        return;
    }

    let (s, e) = app.limits();
    let start = (kind == Kind::Range).then_some(s);
    let active = match app.row {
        Row::Start => Some(0),
        Row::End => Some(1),
        _ => None,
    };
    let mut line = vec![Span::styled("  0 ", th.dim)];
    line.extend(th.slider(start, e, inner.width.saturating_sub(10), active));
    line.push(Span::styled(" 100", th.dim));
    f.render_widget(Paragraph::new(Line::from(line)), slider);

    let mut line = Vec::new();
    if kind == Kind::Range {
        line.push(Span::styled(format!("  {} ", th.case(app.t("desde", "from"))), th.dim));
        line.extend(th.choice(&format!("{s}%"), true, app.row == Row::Start));
    }
    line.push(Span::styled(format!("  {} ", th.case(app.t("hasta", "to"))), th.dim));
    line.extend(th.choice(&format!("{e}%"), true, app.row == Row::End));
    if app.draft.is_some() {
        line.push(Span::styled(th.case(&format!("   {}", app.t("enter aplica · esc cancela", "enter applies · esc cancels"))), th.warn));
    }
    f.render_widget(Paragraph::new(Line::from(line)), values);
}

fn draw_profile(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let focused = app.row == Row::Profile;
    let block = th.block(th.label("󰓅", "perf", app.t("perfil", "profile")), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(state) = &app.status.profile else {
        f.render_widget(Paragraph::new(Span::styled("cpufreq —", th.dim)), inner);
        return;
    };
    let mut line = Vec::new();
    for p in Profile::ALL {
        line.extend(th.choice(p.name(), state.profile == Some(p), focused));
    }
    let turbo = match state.turbo {
        Some(true) => " · turbo",
        _ => "",
    };
    let level = [&state.platform, &state.epp, &state.governor].into_iter().flatten().next().map_or("-", String::as_str);
    let detail = format!(" {level}{turbo}");
    let lines = vec![Line::from(line), Line::from(Span::styled(detail, th.dim))];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_fnlock(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let focused = app.row == Row::Fnlock;
    let block = th.block(th.label("󰌌", "fn", "fn-lock"), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(on) = app.status.fnlock else {
        f.render_widget(Paragraph::new(Span::styled("—", th.dim)), inner);
        return;
    };
    let mut line = th.choice("off", !on, focused);
    line.extend(th.choice("on", on, focused));
    let keys = if on { "F1–F12" } else { app.t("multimedia", "media keys") };
    let hint = Span::styled(th.case(&format!(" {keys} · {}", app.t("espacio", "space"))), th.dim);
    f.render_widget(Paragraph::new(vec![Line::from(line), Line::from(hint)]), inner);
}

fn draw_kbd(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let focused = app.row == Row::Kbd;
    let block = th.block(th.label("󰥻", "kbd", app.t("teclado", "keyboard")), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(k) = &app.status.kbd else { return };
    let style = if focused { th.acc } else { th.current };
    let mut line = vec![Span::raw(" ")];
    line.extend(th.bar(k.level as f64 / k.max.max(1) as f64, inner.width.saturating_sub(8).min(12), style));
    line.push(Span::styled(format!(" {}/{}", k.level, k.max), th.fg));
    let hint = Span::styled(th.case(&format!(" ←→ · {}", app.t("espacio", "space"))), th.dim);
    f.render_widget(Paragraph::new(vec![Line::from(line), Line::from(hint)]), inner);
}

fn draw_thermal(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let block = th.block(th.label("󰔏", "temp", app.t("temperaturas", "thermals")), false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let temp = |c: f64| {
        let style = if c >= 85.0 { th.crit } else if c >= 70.0 { th.warn } else { th.fg };
        Span::styled(format!("{c:.0}°C"), style)
    };
    let t = &app.status.thermals;
    let mut lines = Vec::new();
    if let Some(c) = app.status.cpu_celsius {
        lines.push(Line::from(vec![Span::styled(th.case(" cpu  "), th.dim), temp(c)]));
    }
    if let Some(s) = t.sensors.iter().find(|s| s.chip == "nvme") {
        lines.push(Line::from(vec![Span::styled(th.case(" nvme "), th.dim), temp(s.celsius)]));
    }
    for fan in &t.fans {
        lines.push(Line::from(vec![Span::styled(th.case(" fan  "), th.dim), Span::styled(format!("{} rpm", fan.rpm), th.fg)]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let line = match &app.message {
        Some((text, true)) => Line::from(Span::styled(format!(" {text}"), th.crit)),
        Some((text, false)) => Line::from(vec![Span::styled(" ✓ ", th.acc), Span::styled(text.clone(), th.fg)]),
        None => Line::from(Span::styled(
            th.case(&format!(
                " ↑↓ {}  ←→ {}  enter {}  q {}",
                app.t("mover", "move"),
                app.t("cambiar", "change"),
                app.t("aplicar", "apply"),
                app.t("salir", "quit"),
            )),
            th.dim,
        )),
    };
    f.render_widget(Paragraph::new(line), area);
}

/// Read-only: the fingers fprintd has and the logins that take one (`fierro huella` changes them).
fn draw_huella(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let block = th.block(th.label("󰈷", "fp", app.t("huella", "fingerprint")), false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some((enrolled, pam)) = &app.huella else { return };
    let mut lines = Vec::new();
    if let Some(url) = app.status.fingerprint.as_ref().and_then(|r| r.missing_driver) {
        lines.push(Line::from(Span::styled(th.case(app.t(" falta el driver", " needs a driver")), th.warn)));
        lines.push(Line::from(Span::styled(format!(" {}", url.trim_start_matches("https://")), th.dim)));
    } else if enrolled.fingers.is_empty() {
        lines.push(Line::from(Span::styled(th.case(app.t(" sin dedos", " no fingers")), th.warn)));
        lines.push(Line::from(Span::styled(th.case(app.t(" fierro huella registrar", " fierro huella enroll")), th.dim)));
    } else {
        // "right-index-finger" → "index", "left-thumb" → "thumb ←"
        let names: Vec<String> = enrolled
            .fingers
            .iter()
            .map(|f| {
                let short = f.trim_end_matches("-finger").split('-').next_back().unwrap_or(f);
                if f.starts_with("left") { format!("{short}←") } else { short.to_string() }
            })
            .collect();
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", enrolled.fingers.len()), th.acc),
            Span::styled(names.join(" "), th.fg),
        ]));
        // two logins a line: three fit in the panel's three rows without cutting
        for pair in pam.chunks(2) {
            let mut line = vec![Span::raw(" ")];
            for s in pair {
                let name = s.name.trim_end_matches("-1");
                line.push(Span::styled(format!("{name} "), th.dim));
                line.push(if s.on { Span::styled("✓ ", th.acc) } else { Span::styled("· ", th.dim) });
            }
            lines.push(Line::from(line));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}
