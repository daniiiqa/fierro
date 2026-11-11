# fierro

Laptop hardware control for Linux, from the terminal: the parts of the vendor's
"PC Manager" that matter, for any laptop, a todo fierro.

```
$ fierro status
HUAWEI MDF-XX · BIOS 1.27
battery  43% held at limit · 11.1 W
health   92.4% (51.0 / 55.2 Wh) · 130 cycles
charge   40–70% (casa)
fn-lock  off (media keys)
profile  balance (powersave / balance_performance / turbo on)
thermal  cpu 42°C · fan 2100 rpm
```

Run `fierro` with no arguments for the dashboard: battery and health gauges,
a power graph, the charge-limit slider, profiles, fn-lock, keyboard light and
temperatures — only the panels your laptop actually has.

## What it does

| | |
|---|---|
| **Charge limits** | presets `casa` 40–70 (plugged in all day), `oficina` 70–90, `viaje` 100 — as start + end, end only, or on/off, whatever the firmware offers |
| **Profiles** | `ahorro` / `balance` / `fierro`: ACPI `platform_profile` where there is one, plus governor, EPP and turbo on every core |
| **Fn-lock** | on: F1–F12 work without Fn · off: media keys |
| **Keyboard light** | any `*::kbd_backlight` LED |
| **Battery health** | wear %, cycles, live power draw, time left |
| **Thermals** | every hwmon sensor and fan |
| **Restore** | `fierro restore` reapplies your last limits and profile at login |
| **Fingerprint** | `fierro huella`: the reader, enrolled fingers, enroll/delete (Spanish finger names too), and `pam on|off` for sudo, polkit and SDDM. Spots readers with no stock driver (Goodix GXFP5130 on MateBooks) and says where to get one |

## Hardware

fierro looks for capabilities, not vendors: each feature uses the first interface
the kernel exposes.

| | charge limits | profiles | fn-lock | keyboard light | |
|---|---|---|---|---|---|
| Huawei MateBook (`huawei-wmi`) | start + end | EPP | ✓ | — | ✅ tested |
| ThinkPad (`thinkpad_acpi`) | start + end | platform_profile | — | ✓ | 🧪 untested |
| Lenovo IdeaPad / Legion (`ideapad_laptop`) | conservation on/off | platform_profile | ✓ | ✓ | 🧪 untested |
| ASUS (`asus-wmi`) | end only | platform_profile | — | ✓ | 🧪 untested |
| Framework, Samsung Galaxy Book, others with battery thresholds | end only or start + end | platform_profile or EPP | — | ✓ | 🧪 untested |

Untested rows are built from the kernel drivers' documentation and covered by
tests against fake sysfs trees. If you have one of these laptops, `fierro report`
prints what yours exposes (no serial numbers) — paste it into an issue, it becomes
a test fixture.

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
fierro kbd [LEVEL|next]
fierro health
fierro bar                 waybar custom-module JSON (see dist/waybar-module.jsonc)
fierro restore
fierro report              what this laptop exposes, for bug reports
fierro huella [enroll|delete [FINGER]] [pam on|off]
```

`charge` and `profile` remember what you set in `~/.config/fierro/config.json`;
`--temp` applies a profile without remembering it (for battery-saver scripts).
Set `"lang": "es"` there for the dashboard in Spanish.

`huella pam on` asks for the finger first in sudo and polkit (password after 10 s),
and in SDDM takes the password as always or a finger on Enter with an empty field.
Every line it adds ends in `# fierro huella`; `pam off` removes exactly those, and
the first change keeps a `*.fierro-orig` copy next to each file.

## Look

The dashboard follows the shared TUI style of the discada rice:
`~/.config/fierro/look.json`, or else `~/.config/rola/look.json`.

```json
{ "borders": "auto", "bar": "line", "labels": "icons", "case": "normal",
  "select": "fill", "tint": "color", "transparent": true }
```

borders `auto rounded plain double thick dashed none` · bar `line knob blocks dots ascii` ·
labels `icons short words` · case `normal upper lower` · select `fill bar inverse` ·
tint `color phosphor ink`. Colors come from the kitty theme that `kitty.conf`
includes (wallust's), the same way rola reads them. The dashboard restyles itself
while open when the look, the palette or the theme file changes.

## Hardware notes

**Huawei MateBook D (MDF-XX, BIOS 1.27, Alder Lake)**, the machine fierro grew up on:

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
