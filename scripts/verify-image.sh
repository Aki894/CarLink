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
if python3 -c 'import json,sys; sys.exit(0 if json.load(open(sys.argv[1])).get("app_notices") else 1)' "$mountdir/etc/carlink-build.json"; then
    test -s "$mountdir/usr/local/lib/carlink/app/share/licenses/rust/SOURCE.json"
    test -s "$mountdir/usr/local/lib/carlink/app/share/source/rust/Cargo.lock"
    (cd "$mountdir/usr/local/lib/carlink/app" && sha256sum -c SHA256SUMS)
    cmp "$mountdir/usr/local/bin/carlink" "$mountdir/usr/local/lib/carlink/app/carlink"
fi
test -x "$mountdir/usr/local/bin/carlink"
if [[ "$mode" == current ]]; then
# HiCar resolves FE35 RFCOMM separately from same-UUID ATT records.
test -x "$mountdir/usr/bin/sdptool"
readelf -h "$mountdir/usr/bin/sdptool" | grep -q 'Machine:.*ARM'
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
python3 "$(dirname "$0")/verify-production-runtime.py" "$mountdir"
python3 "$(dirname "$0")/install-boot-profile.py" --root "$mountdir" --verify
for payload in lib/libcarlink_menu.so share/fonts/NotoSansCJKsc-Regular.otf share/source/menu/SOURCE.json share/licenses/menu/OFL.txt forward.py supervisor.py share/source/audio/SOURCE.json share/source/audio/ffmpeg.tar.gz share/licenses/audio/COPYING.LGPLv2.1 SOURCE.json SHA256SUMS share/source/relay.tar.gz share/source/stream.tar.gz \
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
menu=json.load(open(Path(sys.argv[1]).parent/'share/source/menu/SOURCE.json'))
assert menu==build['sources']['menu'], 'Menu source lock mismatch'
import hashlib
font=Path(sys.argv[1]).parent/'share/fonts/NotoSansCJKsc-Regular.otf'
expected_font=next(item['sha256'] for item in menu['dependencies'] if item['name']==font.name)
assert hashlib.sha256(font.read_bytes()).hexdigest()==expected_font, 'Menu font source checksum mismatch'
PY
readelf -h "$receiver/lib/libcarlink_menu.so" | grep -q 'Machine:.*ARM'
readelf -h "$receiver/lib/libcarlink_menu.so" | grep -q 'Flags:.*hard-float ABI'
fi

# Formal images require native auth above. Legacy baseline images may omit it;
# when declared, check its full manifest, ABI and exact source lock below.
python3 - "$mountdir" "$(dirname "$0")" <<'PY'
import hashlib,importlib.util,json,sys
from pathlib import Path
root=Path(sys.argv[1])
build=json.load(open(root/'etc/carlink-build.json'))
expected=build.get('hicar_native_source')
if expected is not None:
    spec=importlib.util.spec_from_file_location('runtime_install',Path(sys.argv[2])/'install-projection-runtime.py')
    installer=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(installer)
    native=root/'usr/local/lib/carlink/hicar-native'
    declared=installer.checksums(native)
    required=['libcarlink_hichain.so','lib/libcrypto.so.3','lib/libmbedcrypto.so.7','lib/libcjson.so.1']
    assert 'sources.json' in declared, 'HiCar native source lock is not checksummed'
    for relative in required:
        assert relative in declared, 'HiCar native runtime missing: '+relative
        installer.armhf(native/relative)
    assert json.load(open(native/'sources.json'))==expected, 'HiCar native source lock mismatch'
    assert hashlib.sha256((native/'libcarlink_hichain.so').read_bytes()).hexdigest()==build['hicar_native_sha256'], 'HiCar native image manifest checksum mismatch'
    for path in native.rglob('*'):
        assert path.stat().st_mode&0o777==(0o755 if path.is_dir() else 0o644), 'HiCar native permissions differ: '+path.relative_to(native).as_posix()
    print('HiCar native runtime checksums, ARM hard-float ABI, source lock and permissions checked')
PY
test -x "$mountdir/usr/sbin/sshd"
sudo grep -q '192.168.77.2/24' "$mountdir/etc/NetworkManager/system-connections/carlink-direct.nmconnection"
grep -q '^CONFIG_USB_CONFIGFS_F_FS=y$' "$mountdir"/boot/config-*
require_payload "$mountdir/lib/modules" module btusb
if [[ "$mode" == --baseline ]]; then
grep -q '^CONFIG_RTW88_8821CU=m$' "$mountdir"/boot/config-*
for module in rtw88_8821cu btusb btrtl; do
    require_payload "$mountdir/lib/modules" module "$module"
done
for fw in rtw88/rtw8821c_fw.bin rtl_bt/rtl8821c_fw.bin rtl_bt/rtl8821c_config.bin; do
    require_payload "$mountdir/lib/firmware" firmware "$fw"
done
fi
require_no_g_serial "$mountdir/etc/modules" "$mountdir/etc/modules-load.d/"
echo "Image structure, app, kernel configuration, modules and firmware checked (mode=$mode); current mode also checks projection/menu runtime/source/checksums. Hardware untested."
