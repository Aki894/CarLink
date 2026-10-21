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
abi_fingerprint() {
    python3 - "$kdir" <<'PY'
import hashlib, json, pathlib, sys
root = pathlib.Path(sys.argv[1])
files = [root / '.config', root / 'Module.symvers']
for directory in ['include/config', 'include/generated', 'arch/arm/include/generated']:
    files.extend(p for p in (root / directory).rglob('*') if p.is_file())
records = {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(set(files))}
print(hashlib.sha256(json.dumps(records, sort_keys=True).encode()).hexdigest())
PY
}
abi_before=$(abi_fingerprint)
echo "Pinned ABI tree fingerprint before host-tool build: $abi_before"
# Do NOT call top-level 'scripts', 'scripts/mod/' or modules_prepare. They
# invoke syncconfig/archheaders, while Armbian deliberately omits syscall.tbl
# from its headers package. Parse the exact top-level Makefile in EXTERNAL
# module mode, where may-sync-config is off, using a custom target with no
# preparation dependencies. Export its canonical flags before building only
# basic/mod through Makefile.build. This includes host scripts/include + LFS
# flags and target ARM ABI/include/compiler flags without hand-maintaining a
# partial copy of the top-level Makefile.
mkdir -p "$work/flags-probe"
printf '# Flags probe only; no module objects.\n' > "$work/flags-probe/Makefile"
cat > "$work/export-kbuild-flags.py" <<'PY'
import json, os, pathlib, sys
names = [
    'KBUILD_HOSTCFLAGS', 'KBUILD_HOSTCXXFLAGS', 'KBUILD_HOSTLDFLAGS', 'KBUILD_HOSTLDLIBS',
    'KBUILD_CFLAGS', 'KBUILD_CPPFLAGS', 'KBUILD_AFLAGS', 'NOSTDINC_FLAGS', 'LINUXINCLUDE',
    'KBUILD_CFLAGS_KERNEL', 'KBUILD_CFLAGS_MODULE', 'KBUILD_AFLAGS_KERNEL', 'KBUILD_AFLAGS_MODULE',
    'KBUILD_LDFLAGS', 'KBUILD_LDFLAGS_MODULE', 'CC_FLAGS_LTO', 'CONFIG_SHELL',
    'HOSTPKG_CONFIG', 'LEX', 'YACC', 'AWK', 'PERL', 'PYTHON3', 'UTS_MACHINE',
    'KBUILD_BUILD_TIMESTAMP',
]
values = {name: os.environ.get(name, '') for name in names}
assert '/scripts/include' in values['KBUILD_HOSTCFLAGS'], 'Missing kernel host include directory'
assert '-D__KERNEL__' in values['KBUILD_CPPFLAGS'], 'Missing kernel target preprocessor flags'
assert '-nostdinc' in values['NOSTDINC_FLAGS'], 'Missing target include isolation'
assert 'arch/arm/include' in values['LINUXINCLUDE'], 'Missing ARM include directories'
pathlib.Path(sys.argv[1]).write_text(json.dumps(values, indent=2) + '\n')
PY
make -C "$kdir" M="$work/flags-probe" ARCH=arm CROSS_COMPILE="$cross" HOSTCC=gcc HOSTCXX=g++ \
    --eval="carlink-export-flags: ; @python3 '$work/export-kbuild-flags.py' '$work/kbuild-flags.json'" \
    carlink-export-flags
[[ $(abi_fingerprint) == "$abi_before" ]] || { echo 'Kbuild flag export modified pinned ABI inputs'; exit 1; }
mapfile -d '' -t kbuild_flags < <(python3 - "$work/kbuild-flags.json" <<'PY'
import json, sys
for key, value in json.load(open(sys.argv[1])).items():
    if value:
        sys.stdout.buffer.write((key + '=' + value).encode() + b'\0')
PY
)
# modpost needs target ARM empty.o/devicetable-offsets.s for ELF layout; these
# are tools' temporary objects, not regenerated kernel configuration/headers.
host_build=(make -C "$kdir" -f scripts/Makefile.build -j2
    "srctree=$kdir" "srcroot=$kdir" "objtree=$kdir"
    ARCH=arm SRCARCH=arm "CROSS_COMPILE=$cross"
    "CC=${cross}gcc" "LD=${cross}ld" "AR=${cross}ar" "NM=${cross}nm"
    "CPP=${cross}gcc -E" "OBJCOPY=${cross}objcopy" "OBJDUMP=${cross}objdump"
    "READELF=${cross}readelf" "STRIP=${cross}strip"
    HOSTCC=gcc HOSTCXX=g++ HOSTLD=ld
    "${kbuild_flags[@]}" CONFIG_SHELL=/bin/bash SHELL=/bin/bash)
"${host_build[@]}" obj=scripts/basic
"${host_build[@]}" obj=scripts/mod
for tool in scripts/basic/fixdep scripts/mod/modpost; do
    test -x "$kdir/$tool"
    file "$kdir/$tool" | grep -q 'x86-64' || { echo "Host utility is not x86_64: $tool"; exit 1; }
done
[[ $(abi_fingerprint) == "$abi_before" ]] || { echo 'Host-tool build modified pinned ABI inputs'; exit 1; }

git init "$work/source"
git -C "$work/source" remote add origin https://github.com/shenmintao/aic8800d80.git
git -C "$work/source" fetch --depth=1 origin "$commit"
git -C "$work/source" checkout --detach FETCH_HEAD
[[ $(git -C "$work/source" rev-parse HEAD) == "$commit" ]]
source_dir="$work/source/drivers/aic8800"
make -C "$source_dir" -j2 ARCH=arm CROSS_COMPILE="$cross" KDIR="$kdir" KVER="$release" HOSTCC=gcc HOSTCXX=g++
[[ $(abi_fingerprint) == "$abi_before" ]] || { echo 'Module build changed pinned ABI inputs'; exit 1; }

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
for firmware_entry in "$work/source/fw/"*; do
    if [[ -d $firmware_entry && $(basename "$firmware_entry") == aic8800* ]]; then
        cp -a "$firmware_entry" "$stage/firmware/"
    elif [[ -f $firmware_entry ]]; then
        cp "$firmware_entry" "$stage/licenses/firmware-root-$(basename "$firmware_entry")"
    else
        echo "Unexpected firmware entry: $firmware_entry"; exit 1
    fi
done
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
