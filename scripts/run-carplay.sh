#!/bin/sh
# Start the independent CarPlay receiver in the foreground; wired SSH stays up.
set -eu
umask 077
root=${CARLINK_CARPLAY_ROOT:-/usr/local/lib/carlink/carplay}
state=${CARLINK_CARPLAY_STATE:-/var/lib/carlink/carplay}
iface=${CARLINK_CARPLAY_WIFI:-wlx002e2d91fc8e}
channel=${CARLINK_CARPLAY_CHANNEL:-149}
country=${CARLINK_CARPLAY_COUNTRY:-CN}
mode=${1:-probe}
case "$mode" in
    probe|web|forward) if [ "$#" -gt 0 ]; then shift; fi ;;
    *) echo 'Usage: run-carplay.sh [probe|web|forward] [receiver options...]' >&2; exit 2 ;;
esac
[ "$(id -u)" = 0 ] || { echo 'Run as root for Bluetooth and hotspot management.' >&2; exit 1; }
for command in nmcli iw bluetoothctl python3; do
    command -v "$command" >/dev/null || { echo "Missing prerequisite: $command" >&2; exit 1; }
done
test -d "/sys/class/net/$iface" || { echo "Wi-Fi interface missing: $iface" >&2; exit 1; }
for file in identity.pk8 certificate.p7b; do
    test -s "$state/mfi/$file" || { echo "Missing local identity file: $state/mfi/$file" >&2; exit 1; }
done
install -d -m 700 "$state/state" /run/carlink-carplay
if [ "$mode" = probe ] || [ "$mode" = forward ]; then
    if [ "$mode" = forward ]; then output="$root/forward.py"; else output="$root/probe.py"; fi
    # iOS wireless audio initialization requires a compatible voice offer even
    # when only AAC media is playing. The probe accepts compressed OXP packets.
    export CARLINK_CARPLAY_VOICE_OPUS=${CARLINK_CARPLAY_VOICE_OPUS:-1}
    export CARLINK_CARPLAY_BUFFERED_AUDIO=${CARLINK_CARPLAY_BUFFERED_AUDIO:-1}
else
    output="$root/bin/oxp-relay"
fi
test -x "$output" || { echo "Missing executable: $output" >&2; exit 1; }
iw reg set "$country"
exec "$root/bin/openxplay-relay" \
    --state-dir "$state/state" --identity-dir "$state/mfi" \
    --wifi-interface "$iface" --wifi-channel "$channel" --network-mode device-hotspot \
    --stream-socket /run/carlink-carplay/media.sock --oxp-relay "$output" \
    --web-root "$root/share/web" --admin-root "$root/share/admin" \
    --plugins-root "$root/share/plugins" --output web "$@"
