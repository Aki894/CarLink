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
echo 'Image structure, app, kernel configuration, modules and firmware checked; hardware untested.'
