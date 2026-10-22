#!/bin/bash
# Structural verification only. This does not boot the board or test the radio.
set -euo pipefail

# -L follows usr-merged /lib and firmware's relative symlinks. Require a
# readable, nonempty regular target and exact names with supported compression.
# Keep find's exit code: unreadable trees and symlink loops are errors.
require_payload() {
    local directory=$1 kind=$2 expected=$3 match
    if [[ "$kind" == module ]]; then
        match=$(find -L "$directory" -type f -readable -size +0c \
            \( -name "$expected.ko" -o -name "$expected.ko.xz" -o -name "$expected.ko.zst" \) \
            -print -quit) || return
    else
        match=$(find -L "$directory" -type f -readable -size +0c \
            \( -path "*/$expected" -o -path "*/$expected.xz" -o -path "*/$expected.zst" \) \
            -print -quit) || return
    fi
    if [[ -z "$match" ]]; then
        echo "Missing readable nonempty $kind: $expected in $directory" >&2
        return 1
    fi
}

require_no_g_serial() {
    local result=0
    grep -R -E '^[[:space:]]*g_serial([[:space:]]|$)' "$@" || result=$?
    case "$result" in
        1) return 0 ;; # A completed search with no matches is the only pass.
        0) echo 'Unexpected g_serial autoload configuration' >&2; return 1 ;;
        *) echo "Could not inspect module autoload configuration (grep exit $result)" >&2; return "$result" ;;
    esac
}

# Allow the fixture test to use the same gates without mounting an image.
if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
    return 0
fi

image=$1
mode=${2:-current}
[[ "$mode" == current || "$mode" == --baseline ]] || { echo 'Unknown image validation mode' >&2; exit 2; }
python3 - "$image" <<'PY'
import sys
with open(sys.argv[1], 'rb') as image:
    image.seek(8196)
    assert image.read(8) == b'eGON.BT0', 'Missing Allwinner SPL header'
    image.seek(510)
    assert image.read(2) == b'\x55\xaa', 'Missing partition signature'
PY
loop=$(sudo losetup --find --show --partscan "$image")
mountdir=$(mktemp -d)
cleanup() {
    if mountpoint -q "$mountdir"; then sudo umount "$mountdir"; fi
    sudo losetup -d "$loop"
    rmdir "$mountdir"
}
trap cleanup EXIT
sudo udevadm settle
sudo mount -o ro "${loop}p1" "$mountdir"
test -f "$mountdir/boot/boot.scr"
test -f "$mountdir/etc/carlink-build.json"
test -x "$mountdir/usr/local/bin/carlink"
if [[ "$mode" == current ]]; then
test -x "$mountdir/usr/local/bin/carlink-carplay"
receiver="$mountdir/usr/local/lib/carlink/carplay"
for name in openxplay-relay oxp-relay carlink-audio; do
    test -x "$receiver/bin/$name"
    readelf -h "$receiver/bin/$name" | grep -q 'Machine:.*ARM'
done
test -x "$receiver/probe.py"
test -x "$receiver/forward.py"
test -x "$receiver/supervisor.py"
test -x "$mountdir/usr/local/bin/carlink-run"
python3 "$(dirname "$0")/install-boot-profile.py" --root "$mountdir" --verify
for payload in forward.py supervisor.py share/source/audio/SOURCE.json share/source/audio/ffmpeg.tar.gz share/licenses/audio/COPYING.LGPLv2.1 SOURCE.json SHA256SUMS share/source/relay.tar.gz share/source/stream.tar.gz \
    share/licenses/relay/LICENSE share/licenses/relay/LICENSE-GPL-3.0 share/licenses/relay/NOTICE \
    share/licenses/stream/LICENSE share/admin/index.html share/web/receiver/index.html; do
    test -s "$receiver/$payload"
done
(cd "$receiver" && sha256sum -c SHA256SUMS)
cmp "$receiver/run-carplay.sh" "$mountdir/usr/local/bin/carlink-carplay"
python3 - "$receiver/SOURCE.json" "$mountdir/etc/carlink-build.json" <<'PY'
import json,sys
receiver,build=[json.load(open(p)) for p in sys.argv[1:]]
for key in ['rust','target','relay','stream']:
    assert receiver[key] == build['sources']['carplay'][key], f'CarPlay image lock mismatch: {key}'
assert receiver['target']=='armv7-unknown-linux-gnueabihf'
from pathlib import Path
audio=json.load(open(Path(sys.argv[1]).parent/'share/source/audio/SOURCE.json'))
for key,value in build['sources']['audio_transcoder'].items():
    assert audio[key]==value, 'Audio source lock mismatch: '+key
assert audio['project_commit']==receiver['project_commit']
PY
fi
test -x "$mountdir/usr/sbin/sshd"
sudo grep -q '192.168.77.2/24' "$mountdir/etc/NetworkManager/system-connections/carlink-direct.nmconnection"
grep -q '^CONFIG_RTW88_8821CU=m$' "$mountdir"/boot/config-*
grep -q '^CONFIG_USB_CONFIGFS_F_FS=y$' "$mountdir"/boot/config-*
for module in rtw88_8821cu btusb btrtl; do
    require_payload "$mountdir/lib/modules" module "$module"
done
for fw in rtw88/rtw8821c_fw.bin rtl_bt/rtl8821c_fw.bin rtl_bt/rtl8821c_config.bin; do
    require_payload "$mountdir/lib/firmware" firmware "$fw"
done
require_no_g_serial "$mountdir/etc/modules" "$mountdir/etc/modules-load.d/"
echo "Image structure, app, kernel configuration, modules and firmware checked (mode=$mode); current mode also checks CarPlay runtime/source/checksums. Hardware untested."
