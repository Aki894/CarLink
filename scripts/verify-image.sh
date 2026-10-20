#!/bin/bash
# Structural verification only. This does not boot the board or test the radio.
set -euo pipefail
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
    test -n "$(find -H "$mountdir/lib/modules" -name "$module.ko*" -print -quit)"
done
for fw in rtw88/rtw8821c_fw.bin rtl_bt/rtl8821c_fw.bin rtl_bt/rtl8821c_config.bin; do
    test -n "$(find -H "$mountdir/lib/firmware" -path "*/$fw*" -print -quit)"
done
! grep -R -E '^[[:space:]]*g_serial([[:space:]]|$)' "$mountdir/etc/modules" "$mountdir/etc/modules-load.d/"
echo 'Image structure, app, kernel configuration, modules and firmware checked; hardware untested.'
