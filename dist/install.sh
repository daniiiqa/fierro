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

if systemctl list-unit-files power-profiles-daemon.service >/dev/null 2>&1; then
    systemctl disable --now power-profiles-daemon || true
    systemctl mask power-profiles-daemon
fi

echo "done: log out and back in so $USER_NAME picks up the fierro group"
