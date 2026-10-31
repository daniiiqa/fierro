# fierro

Huawei MateBook hardware control for Linux, from the terminal — the parts of
Huawei PC Manager that matter, a todo fierro.

```
$ fierro status
HUAWEI MDF-XX (M1010) · BIOS 1.27
battery  43% held at limit · 11.1 W
health   92.4% (51.0 / 55.2 Wh) · 130 cycles
charge   40–70% (casa)
fn-lock  off (media keys)
profile  balance (powersave / balance_performance, turbo on)
thermal  cpu 42°C · fan 2100 rpm
```

Run `fierro` with no arguments for the dashboard: battery and health gauges,
a power graph, the Smart Charge slider, profiles, fn-lock and temperatures.

## What it does

| | |
|---|---|
| **Smart Charge** | Charge limits through `huawei-wmi`, with presets: `casa` 40–70 (plugged in all day), `oficina` 70–90, `viaje` 100 |
| **Profiles** | `ahorro` / `balance` / `fierro`: cpufreq governor + Intel EPP + turbo on every core |
| **Fn-lock** | on: F1–F12 work without Fn · off: media keys |
| **Battery health** | wear %, cycles, live power draw, time left |
| **Thermals** | every hwmon sensor and fan |
| **Restore** | `fierro restore` reapplies your last limits and profile at login |

## Install

```sh
cargo install --path . --root ~/.local
sudo ./dist/install.sh          # once: fierro group + permissions, so fierro never needs root
```

Log out and back in after `install.sh`. It also masks `power-profiles-daemon`,
which would otherwise fight fierro over the CPU settings.

To reapply settings at login without a compositor hook:

```sh
install -Dm644 dist/fierro-restore.service ~/.config/systemd/user/fierro-restore.service
systemctl --user enable fierro-restore
```

## Usage

```
fierro                     dashboard (in a terminal)
fierro status [--json]     everything at once
fierro charge [casa|oficina|viaje|START END]
fierro profile [ahorro|balance|fierro|next] [--temp]
fierro fnlock [on|off|toggle]
fierro health
fierro bar                 waybar custom-module JSON (see dist/waybar-module.jsonc)
fierro restore
```

`charge` and `profile` remember what you set in `~/.config/fierro/config.json`;
`--temp` applies a profile without remembering it (for battery-saver scripts).
Set `"lang": "es"` there for the dashboard in Spanish.

## Look

The dashboard follows the shared TUI style of the discada rice:
`~/.config/fierro/look.json`, or else `~/.config/rola/look.json`.

```json
{ "borders": "auto", "bar": "line", "labels": "icons", "case": "normal",
  "select": "fill", "tint": "color", "transparent": true }
```

borders `auto rounded plain double thick dashed none` · bar `line knob blocks dots ascii` ·
labels `icons short words` · case `normal upper lower` · select `fill bar inverse` ·
tint `color phosphor ink`. Colors come from the terminal palette.

## Hardware notes

Tested on a MateBook MDF-XX (BIOS 1.27, Alder Lake). Other MateBooks with the
`huawei-wmi` driver should work; please open an issue with `fierro status --json`.

- The firmware stores "0 100" (no limit) as "95 100".
- Rewriting the limits makes the EC pause charging for a moment, so
  `restore` only writes what changed.
- No ACPI `platform_profile`, keyboard backlight or fan control is exposed on
  this model; the mic-mute LED is already driven by the kernel (`audio-micmute`).
- The Huawei key is `KEY_CONFIG` (`XF86Tools`): bind it to open `fierro`.

## Development

```sh
cargo test
cargo test preview -- --ignored --nocapture      # render the dashboard as text
FIERRO_SYSFS_ROOT=/path/to/fake/tree fierro      # run against a fake sysfs
```
