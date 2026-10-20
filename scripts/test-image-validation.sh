#!/bin/bash
# Exercise the real verification gates without sudo, loop devices or a firmware build.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
source "$root/scripts/verify-image.sh"
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT

reject() {
    if "$@" > /dev/null 2>&1; then
        echo "Unexpected validation success: $*" >&2
        exit 1
    fi
}

mkdir -p "$fixture/usr/lib/modules/kernel" "$fixture/usr/lib/firmware/rtw88" \
    "$fixture/etc/modules-load.d"
ln -s usr/lib "$fixture/lib"
# These must be real symlinks: MSYS users need MSYS=winsymlinks:nativestrict.
test -L "$fixture/lib"
modules="$fixture/lib/modules"
firmware="$fixture/lib/firmware"

for suffix in '' .xz .zst; do
    printf payload > "$modules/kernel/test.ko$suffix"
    require_payload "$modules" module test
    rm "$modules/kernel/test.ko$suffix"
    printf payload > "$firmware/rtw88/test.bin$suffix"
    require_payload "$firmware" firmware rtw88/test.bin
    rm "$firmware/rtw88/test.bin$suffix"
done

# Name-only matches, empty files and dangling links must not masquerade as payloads.
mkdir "$modules/kernel/test.ko" "$firmware/rtw88/test.bin"
reject require_payload "$modules" module test
reject require_payload "$firmware" firmware rtw88/test.bin
rmdir "$modules/kernel/test.ko" "$firmware/rtw88/test.bin"
touch "$modules/kernel/test.ko" "$firmware/rtw88/test.bin"
reject require_payload "$modules" module test
reject require_payload "$firmware" firmware rtw88/test.bin
rm "$modules/kernel/test.ko" "$firmware/rtw88/test.bin"
printf payload > "$modules/kernel/test.ko.bak"
printf payload > "$firmware/rtw88/test.bin.bak"
reject require_payload "$modules" module test
reject require_payload "$firmware" firmware rtw88/test.bin
ln -s missing "$modules/kernel/test.ko"
ln -s missing "$firmware/rtw88/test.bin"
reject require_payload "$modules" module test
reject require_payload "$firmware" firmware rtw88/test.bin
rm "$modules/kernel/test.ko" "$firmware/rtw88/test.bin"

# Real relative symlinks inside firmware and modules must continue to work.
printf payload > "$modules/kernel/shared"
printf payload > "$firmware/shared"
# Create links through physical paths (MSYS otherwise rewrites targets through /lib).
ln -s shared "$fixture/usr/lib/modules/kernel/test.ko"
ln -s ../shared "$fixture/usr/lib/firmware/rtw88/test.bin"
require_payload "$modules" module test
require_payload "$firmware" firmware rtw88/test.bin
reject require_payload "$fixture/no-such-directory" module test

printf '# g_serial is deliberately absent\n' > "$fixture/etc/modules"
require_no_g_serial "$fixture/etc/modules" "$fixture/etc/modules-load.d"
printf '  g_serial\n' > "$fixture/etc/modules-load.d/usb.conf"
reject require_no_g_serial "$fixture/etc/modules" "$fixture/etc/modules-load.d"
rm "$fixture/etc/modules-load.d/usb.conf"
reject require_no_g_serial "$fixture/missing" "$fixture/etc/modules-load.d"

echo 'Image validation fixtures: exact payload names, content, symlinks and grep errors checked.'
