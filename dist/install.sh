#!/bin/sh
# One-time system setup so fierro runs without root:
#   sudo ./dist/install.sh [user]
# Creates the `fierro` group, adds the user to it, installs the boot-time
# permission helper, and masks power-profiles-daemon (it fights fierro's profiles).
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "run with sudo: sudo $0" >&2
    exit 1
fi

USER_NAME=${1:-${SUDO_USER:-}}
if [ -z "$USER_NAME" ]; then
    echo "usage: sudo $0 <user>" >&2
    exit 1
fi

DIST=$(cd "$(dirname "$0")" && pwd)

groupadd -f fierro
usermod -aG fierro "$USER_NAME"
install -Dm755 "$DIST/fierro-perms" /usr/lib/fierro/fierro-perms
install -Dm644 "$DIST/fierro-perms.service" /etc/systemd/system/fierro-perms.service
systemctl daemon-reload
systemctl enable --now fierro-perms

# power-profiles-daemon: with another desktop on this machine (KDE, GNOME…) its
# battery menu needs it, so it stays on and fierro sets profiles through it
# (unmasked again if an older fierro masked it). With only Hyprland it would
# just fight fierro's profiles, so it's masked.
other_desktop() {
    for f in /usr/share/wayland-sessions/*.desktop /usr/share/xsessions/*.desktop; do
        [ -e "$f" ] || continue
        case "$(basename "$f")" in hyprland*) ;; *) return 0 ;; esac
    done
    return 1
}
if systemctl list-unit-files power-profiles-daemon.service >/dev/null 2>&1; then
    if other_desktop; then
        systemctl unmask power-profiles-daemon 2>/dev/null || true
        systemctl enable --now power-profiles-daemon || true
    else
        systemctl disable --now power-profiles-daemon || true
        systemctl mask power-profiles-daemon
    fi
fi

echo "done: log out and back in so $USER_NAME picks up the fierro group"
