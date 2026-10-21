#!/bin/bash
# Cross-build only from a successfully built, hash-pinned kernel package.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
lock="$repo/drivers/aic8800d80/sources.json"
if [[ ${1:-} == --inspect ]]; then
    exec python3 "$repo/drivers/aic8800d80/check-headers.py" "${2:?evidence directory or zip required}"
fi
variant=${1:?usage: build-aic-driver.sh main|mcu1 evidence-directory output-directory}
evidence=$(realpath "${2:?evidence directory required}")
output=$(realpath -m "${3:?output directory required}")
[[ $variant == main || $variant == mcu1 ]] || { echo 'Invalid driver variant'; exit 1; }
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || { echo 'Build on an x86_64 Linux Actions runner'; exit 1; }
python3 "$repo/drivers/aic8800d80/check-headers.py" "$evidence"
readarray -t settings < <(python3 - "$lock" "$variant" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
for value in [m['variants'][sys.argv[2]], m['kernel_release'], m['headers_package']]:
    print(value)
PY
)
commit=${settings[0]}
release=${settings[1]}
headers_package=${settings[2]}
work="$repo/.build/aic-$variant"
mkdir -p "$work" "$output"
[[ ! -e $work/source && ! -e $work/headers ]] || { echo 'Use a fresh per-variant build directory'; exit 1; }
exec > >(tee "$work/build.log") 2>&1
headers=$(find "$evidence" -type f -name "$headers_package" -print -quit)
dpkg-deb -x "$headers" "$work/headers"
kdir="$work/headers/usr/src/linux-headers-$release"
test -s "$kdir/Module.symvers"
test -s "$kdir/include/config/auto.conf"
test -s "$kdir/.config"
grep -qx '#define UTS_RELEASE "'"$release"'"' "$kdir/include/generated/utsrelease.h"
cross=arm-linux-gnueabihf-
[[ $(${cross}gcc -dumpmachine) == arm-linux-gnueabihf ]]
[[ $(${cross}gcc -dumpversion | cut -d. -f1) == 14 ]] || { echo 'This kernel was built with GCC 14; use Debian Trixie cross tools'; exit 1; }
[[ $(gcc -dumpversion | cut -d. -f1) == 14 ]] || { echo 'Expected GCC 14 host compiler'; exit 1; }

# Armbian headers contain sources for host utilities, but no usable x86
# fixdep/modpost executables. Build them on the runner; never run an ARM ELF
# tool or replace the ABI-defining .config/generated headers/Module.symvers.
for tool in scripts/basic/fixdep scripts/mod/modpost scripts/mod/mk_elfconfig scripts/genksyms/genksyms; do
    if [[ -f $kdir/$tool ]]; then
        if file "$kdir/$tool" | grep -q 'ELF'; then
            rm -- "$kdir/$tool"
        fi
    fi
done
abi_before=$(sha256sum "$kdir/Module.symvers" "$kdir/.config" "$kdir/include/config/auto.conf" "$kdir/include/generated/utsrelease.h")
make -C "$kdir" ARCH=arm CROSS_COMPILE="$cross" HOSTCC=gcc HOSTCXX=g++ -j2 scripts
# Some header packages' scripts target omits modpost; explicitly build this
# exact supplied subdirectory if needed, rather than borrowing host headers.
if [[ ! -x $kdir/scripts/mod/modpost ]]; then
    make -C "$kdir" ARCH=arm CROSS_COMPILE="$cross" HOSTCC=gcc HOSTCXX=g++ -j2 scripts/mod/
fi
for tool in scripts/basic/fixdep scripts/mod/modpost; do
    test -x "$kdir/$tool"
    file "$kdir/$tool" | grep -q 'x86-64' || { echo "Host utility is not x86_64: $tool"; exit 1; }
done
[[ $(sha256sum "$kdir/Module.symvers" "$kdir/.config" "$kdir/include/config/auto.conf" "$kdir/include/generated/utsrelease.h") == "$abi_before" ]] || { echo 'Header preparation modified pinned ABI inputs'; exit 1; }

git init "$work/source"
git -C "$work/source" remote add origin https://github.com/shenmintao/aic8800d80.git
git -C "$work/source" fetch --depth=1 origin "$commit"
git -C "$work/source" checkout --detach FETCH_HEAD
[[ $(git -C "$work/source" rev-parse HEAD) == "$commit" ]]
source_dir="$work/source/drivers/aic8800"
make -C "$source_dir" -j2 ARCH=arm CROSS_COMPILE="$cross" KDIR="$kdir" KVER="$release" HOSTCC=gcc HOSTCXX=g++
[[ $(sha256sum "$kdir/Module.symvers" "$kdir/.config" "$kdir/include/config/auto.conf" "$kdir/include/generated/utsrelease.h") == "$abi_before" ]] || { echo 'Module build changed pinned ABI inputs'; exit 1; }

stage="$work/package"
mkdir -p "$stage/modules/$release/extra/aic8800" "$stage/firmware" "$stage/udev" "$stage/licenses"
for module in aic_load_fw aic8800_fdrv aic_zlp_quirk; do
    built="$source_dir/$module/$module.ko"
    test -s "$built"
    ${cross}readelf -h "$built" | tee "$work/$module-elf.log"
    ${cross}readelf -h "$built" | grep -q 'Class:.*ELF32'
    ${cross}readelf -h "$built" | grep -q 'Machine:.*ARM'
    vermagic=$(modinfo -F vermagic "$built")
    [[ $vermagic == "$release "* ]] || { echo "Wrong vermagic: $vermagic"; exit 1; }
    modinfo "$built" > "$work/$module-modinfo.log"
    install -m 0644 "$built" "$stage/modules/$release/extra/aic8800/"
done
test -d "$work/source/fw/aic8800D80"
test -s "$work/source/LICENSE"
# Preserve each pinned variant's entire firmware set and supplied notices.
# Do not execute the third-party installer or alter firmware/calibration data.
cp -a "$work/source/fw/." "$stage/firmware/"
cp "$work/source/LICENSE" "$stage/licenses/aic8800-source-LICENSE"
find "$work/source/fw" -type f \( -iname '*license*' -o -iname '*copying*' -o -iname '*copyright*' \) -print > "$work/firmware-license-files.log"
install -m 0644 "$repo/drivers/aic8800d80/90-carlink-aic8800.rules" "$stage/udev/"
install -m 0755 "$repo/drivers/aic8800d80/install.sh" "$stage/install.sh"
python3 - "$stage" "$lock" "$variant" "$headers" "$repo" <<'PY'
import hashlib, json, pathlib, subprocess, sys
root, lock_path, variant, headers, repo = sys.argv[1:]
root = pathlib.Path(root)
lock = json.load(open(lock_path))
for p in root.rglob('*'):
    if p.is_symlink():
        raise SystemExit('Payload must contain no symbolic links: ' + str(p))
m = dict(schema_version=1, variant=variant, driver_repository=lock['repository'],
         driver_commit=lock['variants'][variant], kernel_release=lock['kernel_release'],
         kernel_evidence_run=lock['kernel_evidence_run'], kernel_evidence_artifact=lock['kernel_evidence_artifact'],
         headers_sha256=lock['headers_sha256'], symvers_sha256=lock['symvers_sha256'],
         kernel_config_sha256=lock['kernel_config_sha256'],
         cross_compiler=subprocess.check_output(['arm-linux-gnueabihf-gcc','--version'],text=True).splitlines()[0],
         builder_commit=subprocess.check_output(['git','-C',repo,'rev-parse','HEAD'],text=True).strip(),
         bluetooth_driver='standard btusb with device-scoped aic_zlp_quirk',
         hardware_validation='not_tested',
         firmware_provenance='Unmodified fw/ files from driver_repository at driver_commit; upstream licensing metadata preserved')
for group in ['modules', 'firmware', 'udev']:
    m[group] = sorted(p.relative_to(root).as_posix() for p in (root/group).rglob('*') if p.is_file())
assert len(m['modules']) == 3 and m['firmware'] and m['udev']
m['file_sha256'] = {p.relative_to(root).as_posix():hashlib.sha256(p.read_bytes()).hexdigest()
                    for p in sorted(root.rglob('*')) if p.is_file()}
(root/'driver-build.json').write_text(json.dumps(m,indent=2)+'\n')
with (root/'SHA256SUMS').open('w') as out:
    for p in sorted(root.rglob('*')):
        if p.is_file() and p.name != 'SHA256SUMS':
            out.write(hashlib.sha256(p.read_bytes()).hexdigest()+'  '+p.relative_to(root).as_posix()+'\n')
PY
(cd "$stage" && sha256sum --check SHA256SUMS)
tar -czf "$output/driver.tar.gz" -C "$stage" modules firmware udev licenses install.sh driver-build.json SHA256SUMS
cp "$stage/driver-build.json" "$output/driver-build.json"
(cd "$output" && sha256sum driver.tar.gz driver-build.json > SHA256SUMS && sha256sum --check SHA256SUMS)
echo "Built $variant for $release. Hardware validation has not been performed."
