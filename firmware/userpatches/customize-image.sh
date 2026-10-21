#!/bin/bash
set -euo pipefail
[[ "${3:-}" == orangepizero ]] || { echo 'Unexpected board' >&2; exit 1; }
install -m 0755 /tmp/overlay/carlink /usr/local/bin/carlink
install -m 0644 /tmp/overlay/carlink-build.json /etc/carlink-build.json
mkdir -p /usr/local/lib/carlink/carplay
cp -a /tmp/overlay/carplay/. /usr/local/lib/carlink/carplay/
chmod 0755 /usr/local/lib/carlink/carplay/bin/openxplay-relay \
    /usr/local/lib/carlink/carplay/bin/oxp-relay /usr/local/lib/carlink/carplay/probe.py
install -m 0755 /tmp/overlay/carplay/run-carplay.sh /usr/local/bin/carlink-carplay
mkdir -p /usr/local/lib/carlink/carplay/share/plugins
install -d -m 0700 /var/lib/carlink/carplay /var/lib/carlink/carplay/mfi
(cd /usr/local/lib/carlink/carplay && sha256sum -c SHA256SUMS)

mkdir -p /etc/NetworkManager/system-connections /etc/systemd/journald.conf.d
install -m 0600 /tmp/overlay/carlink-direct.nmconnection \
    /etc/NetworkManager/system-connections/carlink-direct.nmconnection
install -m 0644 /tmp/overlay/carlink-journal.conf \
    /etc/systemd/journald.conf.d/carlink.conf
mkdir -p /var/log/journal
systemd-tmpfiles --create --prefix /var/log/journal

# Defensive cleanup if the board defaults wrote g_serial before our extension.
for file in /etc/modules /etc/modules-load.d/*.conf; do
    [[ -f "$file" ]] || continue
    sed -i '/^[[:space:]]*g_serial\([[:space:]].*\)\?$/d' "$file"
done
mkdir -p /etc/modprobe.d
printf 'blacklist g_serial\n' > /etc/modprobe.d/carlink-gadget.conf
install -m 0644 /tmp/overlay/carlink-aic.conf /etc/modprobe.d/carlink-aic.conf

# Keep first-login password rotation and SSH host-key generation from Armbian.
systemctl enable ssh
systemctl disable hostapd.service 2>/dev/null || true
ln -sf /usr/share/zoneinfo/Asia/Hong_Kong /etc/localtime
printf 'Asia/Hong_Kong\n' > /etc/timezone
