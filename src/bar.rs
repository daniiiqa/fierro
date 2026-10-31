//! `fierro bar`: one line of waybar custom-module JSON (return-type json).

use serde_json::json;

use fierro::hw::profile::Profile;
use fierro::status::Status;


const LEVELS: [&str; 10] = ["󰁺", "󰁻", "󰁼", "󰁽", "󰁾", "󰁿", "󰂀", "󰂁", "󰂂", "󰁹"];

pub fn render(s: &Status, es: bool) -> String {
    let t = |a: &'static str, b: &'static str| if es { a } else { b };
    let Some(b) = &s.battery else {
        return json!({ "text": "" }).to_string();
    };
    let profile = s.profile.as_ref().and_then(|p| p.profile);

    let icon = if s.held {
        "󰚥"
    } else if b.is_charging() {
        "󰂄"
    } else {
        LEVELS[(b.capacity.min(99) / 10) as usize]
    };
    let glyph = match profile {
        Some(Profile::Ahorro) => "󰌪",
        Some(Profile::Balance) => "󰾅",
        Some(Profile::Fierro) => "󰓅",
        None => "󰾆",
    };
    let mut text = format!("{icon} {}%  {glyph}", b.capacity);
    if let Some(c) = s.cpu_celsius {
        text += &format!(" {c:.0}°");
    }

    let mut class = vec![b.status.to_lowercase().replace(' ', "-")];
    if s.held {
        class.push("held".into());
    }
    if let Some(p) = profile {
        class.push(p.name().into());
    }
    if b.status == "Discharging" {
        match b.capacity {
            0..=10 => class.push("critical".into()),
            11..=20 => class.push("low".into()),
            _ => {}
        }
    }

    let state = if s.held {
        t("detenida en el límite", "held at limit")
    } else {
        match b.status.as_str() {
            "Charging" => t("cargando", "charging"),
            "Discharging" => t("con batería", "on battery"),
            _ => t("conectada", "plugged in"),
        }
    };
    let mut tooltip = vec![format!("{} {}% · {state}", t("Batería", "Battery"), b.capacity)];
    if let Some(c) = s.describe_charge() {
        tooltip.push(format!("{}: {c}", t("Carga", "Charge")));
    }
    tooltip.push(format!("{}: {:.1}%", t("Salud", "Health"), b.health_pct));
    tooltip.push(format!("{}: {}", t("Perfil", "Profile"), profile.map_or("custom", Profile::name)));
    if let Some(c) = s.cpu_celsius {
        let fans: String = s.thermals.fans.iter().map(|f| format!(" · fan {} rpm", f.rpm)).collect();
        tooltip.push(format!("CPU {c:.0}°C{fans}"));
    }

    json!({
        "text": text,
        "alt": profile.map_or("custom", Profile::name),
        "class": class,
        "percentage": b.capacity,
        "tooltip": tooltip.join("\n"),
    })
    .to_string()
}
