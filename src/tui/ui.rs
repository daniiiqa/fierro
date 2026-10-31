use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Sparkline};
use ratatui::Frame;

use fierro::hw::charge::Preset;
use fierro::hw::profile::Profile;

use super::{App, Row};

pub fn draw(f: &mut Frame, app: &App) {
    let th = &app.theme;
    if let Some(bg) = th.background() {
        f.render_widget(Block::new().style(bg), f.area());
    }
    let area = f.area().inner(Margin { horizontal: 1, vertical: 0 });
    let [header, battery, charge, bottom, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(7),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, app, header);
    draw_battery(f, app, battery);
    draw_charge(f, app, charge);
    let [profile, fnlock, thermal] =
        Layout::horizontal([Constraint::Fill(3), Constraint::Fill(2), Constraint::Fill(3)]).areas(bottom);
    draw_profile(f, app, profile);
    draw_fnlock(f, app, fnlock);
    draw_thermal(f, app, thermal);
    draw_footer(f, app, footer);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let m = &app.status.machine;
    let line = Line::from(vec![
        Span::styled(th.case(" fierro "), th.acc),
        Span::styled(format!(" {} {} · BIOS {}", m.vendor, m.product, m.bios), th.dim),
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

    let current = if app.draft.is_some() { None } else { app.status.preset };
    let mut line = Vec::new();
    for p in Preset::ALL {
        line.extend(th.choice(p.name(), current == Some(p), app.row == Row::Preset));
    }
    if current.is_none() {
        line.extend(th.choice(app.t("a mano", "custom"), true, app.row == Row::Preset));
    }
    f.render_widget(Paragraph::new(Line::from(line)), presets);

    let (s, e) = app.limits();
    let active = match app.row {
        Row::Start => Some(0),
        Row::End => Some(1),
        _ => None,
    };
    let mut line = vec![Span::styled("  0 ", th.dim)];
    line.extend(th.slider(s, e, inner.width.saturating_sub(10), active));
    line.push(Span::styled(" 100", th.dim));
    f.render_widget(Paragraph::new(Line::from(line)), slider);

    let mut line = Vec::new();
    line.push(Span::styled(format!("  {} ", th.case(app.t("desde", "from"))), th.dim));
    line.extend(th.choice(&format!("{s}%"), true, app.row == Row::Start));
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
    let detail = format!(" {}{turbo}", state.epp.as_deref().unwrap_or(&state.governor));
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
