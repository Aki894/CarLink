#!/usr/bin/env bash
# Build only in Linux CI. This encoder renders the local device menu, never phone video.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
work="$root/.build/menu"
out="$root/dist/menu"
[[ ! -e "$work" && ! -e "$out" ]] || { echo 'Menu build/output directory already exists' >&2; exit 1; }
mkdir -p "$work/deps" "$out/lib" "$out/bin" "$out/share/fonts" "$out/share/source/menu" "$out/share/licenses/menu"
python3 - "$root/sources.lock.json" "$work/deps" <<'PY'
import hashlib, json, pathlib, sys, urllib.request
lock=json.load(open(sys.argv[1]))['menu']
root=pathlib.Path(sys.argv[2])
for entry in lock['dependencies']:
    assert '/' not in entry['name'] and '..' not in entry['name']
    data=urllib.request.urlopen(entry['url'],timeout=120).read()
    assert hashlib.sha256(data).hexdigest()==entry['sha256'], 'Menu source checksum differs: '+entry['name']
    (root/entry['name']).write_bytes(data)
PY
for compiler in gcc arm-linux-gnueabihf-gcc; do command -v "$compiler" >/dev/null; done
flags=(-O2 -std=c99 -fPIC -fvisibility=hidden -ffunction-sections -fdata-sections -I"$work/deps")
arm_flags=(-march=armv7-a -mcpu=cortex-a7 -mfloat-abi=hard -mfpu=neon-vfpv4)
gcc "${flags[@]}" -shared "$root/receiver/menu/menu.c" -o "$work/libcarlink_menu_host.so" -Wl,--gc-sections -lm
gcc "${flags[@]}" -DCARLINK_MENU_TEST "$root/receiver/menu/menu.c" -o "$work/menu-test-host" -lm
arm-linux-gnueabihf-gcc "${flags[@]}" "${arm_flags[@]}" -shared "$root/receiver/menu/menu.c" -o "$out/lib/libcarlink_menu.so" -Wl,--gc-sections -lm
arm-linux-gnueabihf-gcc "${flags[@]}" "${arm_flags[@]}" -DCARLINK_MENU_TEST "$root/receiver/menu/menu.c" -o "$work/menu-test-arm" -lm
install -m 0755 "$work/menu-test-arm" "$out/bin/carlink-menu-selftest"
arm-linux-gnueabihf-strip "$out/lib/libcarlink_menu.so"
arm-linux-gnueabihf-readelf -h -A "$out/lib/libcarlink_menu.so" > "$out/menu.elf.txt"
grep -q 'Class:.*ELF32' "$out/menu.elf.txt"
grep -q 'Machine:.*ARM' "$out/menu.elf.txt"
grep -q 'Tag_ABI_VFP_args:.*VFP registers' "$out/menu.elf.txt"
cp "$work/deps/NotoSansCJKsc-Regular.otf" "$out/share/fonts/"
cp "$work/deps/minih264e.h" "$work/deps/stb_truetype.h" "$root/receiver/menu/menu.c" "$root/scripts/build-menu.sh" "$out/share/source/menu/"
cp "$work/deps/OFL.txt" "$work/deps/stb-LICENSE" "$out/share/licenses/menu/"
head -n 9 "$work/deps/minih264e.h" > "$out/share/licenses/menu/minih264-CC0.txt"
cp "$root/LICENSE" "$out/share/licenses/menu/CarLink-MIT.txt"
python3 - "$root/sources.lock.json" "$out/share/source/menu/SOURCE.json" <<'PY'
import json,sys
with open(sys.argv[2],'w') as f:json.dump(json.load(open(sys.argv[1]))['menu'],f,indent=2);f.write('\n')
PY
python3 "$root/scripts/test-manager-menu.py" "$work/menu-test-host" "$work/menu-test-arm" "$out/share/fonts/NotoSansCJKsc-Regular.otf" "$work" | tee "$out/menu-validation.txt"
version=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["application"]["rust"])' "$root/sources.lock.json")
CARLINK_MENU_LIBRARY="$work/libcarlink_menu_host.so" CARLINK_MENU_FONT="$out/share/fonts/NotoSansCJKsc-Regular.otf" \
    cargo +"$version" test --locked --manifest-path "$root/app/Cargo.toml" manager_menu:: -- --nocapture
(cd "$out" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS)
