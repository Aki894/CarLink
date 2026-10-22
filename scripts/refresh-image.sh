#!/bin/bash
# Reuse the proven SPL, bootloader and kernel; install verified CI applications.
set -euo pipefail
project=$(cd "$(dirname "$0")/.." && pwd)
python=${PYTHON:-python3}

unpack_driver() {
    "$python" - "$@" <<'PY'
import hashlib, json, pathlib, re, shutil, sys, tarfile, tempfile, unittest

def checked_name(name):
    # Only POSIX relative names; no Windows drive, traversal, repeated separators.
    if not name or name.startswith('/') or '\\' in name or ':' in name:
        raise ValueError('unsafe archive path: ' + repr(name))
    name = name.removesuffix('/')
    parts = name.split('/')
    if any(p in ('', '.', '..') for p in parts):
        raise ValueError('unsafe archive path: ' + repr(name))
    if parts[0] not in ('modules', 'firmware', 'udev', 'licenses', 'driver-build.json', 'install.sh', 'SHA256SUMS'):
        raise ValueError('unexpected archive namespace: ' + name)
    if parts[0] in ('driver-build.json', 'install.sh', 'SHA256SUMS') and len(parts) != 1:
        raise ValueError('metadata must be a root file: ' + name)
    return pathlib.PurePosixPath(name)

def inspect(archive):
    seen, members, size = set(), [], 0
    for item in archive:
        path = checked_name(item.name)
        if str(path) in seen:
            raise ValueError('duplicate archive path: ' + str(path))
        seen.add(str(path))
        if not (item.isfile() or item.isdir()):
            raise ValueError('archive links/devices are not permitted: ' + item.name)
        size += item.size
        if len(seen) > 10000 or size > 256 * 1024 * 1024:
            raise ValueError('driver archive exceeds bounded size/count')
        members.append(item)
    return members

def extract(archive_path, destination):
    with tarfile.open(archive_path, 'r:gz') as archive:
        members = inspect(archive)
        # Explicit copy avoids tar ownership, permissions and link semantics.
        for item in members:
            out = destination.joinpath(*checked_name(item.name).parts)
            if item.isdir():
                out.mkdir(parents=True, exist_ok=True)
            else:
                out.parent.mkdir(parents=True, exist_ok=True)
                with archive.extractfile(item) as src, out.open('xb') as dst:
                    shutil.copyfileobj(src, dst)
    manifest = json.loads((destination / 'driver-build.json').read_text())
    if not re.fullmatch(r'[A-Za-z0-9._+-]+', manifest['kernel_release']):
        raise ValueError('invalid kernel release')
    if manifest['variant'] not in ('main', 'mcu1'):
        raise ValueError('invalid driver variant')
    if not re.fullmatch(r'[0-9a-f]{40}', manifest['driver_commit']):
        raise ValueError('driver_commit must be an exact source commit')
    for key, prefix in [('modules', 'modules'), ('firmware', 'firmware')]:
        paths = manifest[key]
        if not isinstance(paths, list) or not paths or len(paths) != len(set(paths)):
            raise ValueError('missing or duplicate manifest entries: ' + key)
        for entry in paths:
            path = checked_name(entry)
            if path.parts[0] != prefix or not (destination / path).is_file() or not (destination / path).stat().st_size:
                raise ValueError('missing or invalid manifest payload: ' + entry)
            if key == 'modules' and (len(path.parts) != 5 or path.parts[1] != manifest['kernel_release'] or path.parts[2:4] != ('extra','aic8800') or path.suffix != '.ko'):
                raise ValueError('module is not for the declared kernel: ' + entry)
            if key == 'firmware' and (len(path.parts) < 3 or not path.parts[1].startswith('aic8800')):
                raise ValueError('firmware must remain in the AIC namespace: ' + entry)
    if manifest.get('udev') != ['udev/90-carlink-aic8800.rules']:
        raise ValueError('unexpected udev rules manifest')
    rules=destination/'udev/90-carlink-aic8800.rules'
    if not rules.is_file() or not rules.stat().st_size:
        raise ValueError('missing udev rules')
    actual = {p.relative_to(destination).as_posix() for prefix in ('modules','firmware','udev') for p in (destination / prefix).rglob('*') if p.is_file()}
    if actual != set(manifest['modules'] + manifest['firmware'] + manifest['udev']):
        raise ValueError('archive contains undeclared module/firmware/udev files')
    return manifest

if sys.argv[1:] == ['--self-test']:
    class Guards(unittest.TestCase):
        def test_names(self):
            for bad in ('../escape','/etc/passwd','modules/../escape','C:/x','modules\\x','modules//x','./modules/x'):
                with self.assertRaises(ValueError): checked_name(bad)
            self.assertEqual(str(checked_name('firmware/aic8800/fw.bin')), 'firmware/aic8800/fw.bin')
        def test_archive_types_and_duplicates(self):
            one = tarfile.TarInfo('firmware/aic8800/fw.bin')
            link = tarfile.TarInfo('firmware/aic8800/link'); link.type = tarfile.SYMTYPE
            with self.assertRaises(ValueError): inspect([one, one])
            with self.assertRaises(ValueError): inspect([link])
            huge = tarfile.TarInfo('modules/huge'); huge.size = 257 * 1024 * 1024
            with self.assertRaises(ValueError): inspect([huge])
        def test_valid_package(self):
            import io
            manifest = {'kernel_release':'6.18.1-current-sunxi','variant':'main','driver_commit':'a'*40,
                'modules':['modules/6.18.1-current-sunxi/extra/aic8800/test.ko'], 'firmware':['firmware/aic8800/fw.bin'], 'udev':['udev/90-carlink-aic8800.rules']}
            with tempfile.TemporaryDirectory() as temp:
                temp = pathlib.Path(temp); archive = temp/'driver.tar.gz'
                with tarfile.open(archive,'w:gz') as tar:
                    for name, data in [('driver-build.json',json.dumps(manifest).encode()),(manifest['modules'][0],b'ko'),(manifest['firmware'][0],b'fw'),(manifest['udev'][0],b'# rules')]:
                        item=tarfile.TarInfo(name); item.size=len(data); tar.addfile(item,io.BytesIO(data))
                self.assertEqual(extract(archive,temp/'out'),manifest)
    unittest.main(argv=['archive-guards'])
else:
    extract(pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]))
PY
}

if [[ "${1:-}" == --self-test ]]; then
    unpack_driver --self-test
    exit
fi
[[ $# == 3 ]] || { echo 'Usage: refresh-image.sh BASE_DIRECTORY OUTPUT_DIRECTORY DRIVER_DIRECTORY' >&2; exit 2; }
[[ $EUID == 0 ]] || { echo 'Image mounting requires root' >&2; exit 2; }
[[ ${BASE_RUN_ID:-} =~ ^[0-9]+$ && ${REFRESH_COMMIT:-} =~ ^[0-9a-f]{40}$ ]]
[[ -z ${DRIVER_RUN_ID:-} || ${DRIVER_RUN_ID} =~ ^[0-9]+$ ]]
[[ ${DRIVER_VARIANT:-main} == main || ${DRIVER_VARIANT:-main} == mcu1 ]]
export DISABLE_ONBOARD_WIFI=${DISABLE_ONBOARD_WIFI:-false}
[[ "$DISABLE_ONBOARD_WIFI" == false || "$DISABLE_ONBOARD_WIFI" == true ]]
[[ "$DISABLE_ONBOARD_WIFI" == false || -n ${DRIVER_RUN_ID:-} ]] || {
    echo 'Disabling XR819 requires an AIC driver run' >&2; exit 2;
}
base=$(realpath "$1")
mkdir -p "$2"
output=$(realpath "$2")
driver_dir=$(realpath -m "$3")
mkdir -p "$output/image" "$output/evidence"
work=$(mktemp -d)
loop=''
cleanup() {
    if mountpoint -q "$work/root"; then umount "$work/root"; fi
    if [[ -n "$loop" ]]; then losetup -d "$loop"; fi
    rm -rf "$work"
}
trap cleanup EXIT

# Validate checksum metadata before decompression and leave the base artifact untouched.
verify_checksums() {
    "$python" - "$1" "$2" <<'PY'
import hashlib, pathlib, re, sys
directory=pathlib.Path(sys.argv[1]); required=sys.argv[2]; found=False
for line in (directory/'SHA256SUMS').read_text().splitlines():
    match=re.fullmatch(r'([0-9a-fA-F]{64}) [ *]([^/\\]+)',line)
    if not match or match[2] in ('.','..'): raise SystemExit('Unsafe SHA256SUMS entry')
    path=directory/match[2]
    if not path.is_file() or path.is_symlink(): raise SystemExit('Checksum payload missing or linked')
    h=hashlib.sha256()
    with path.open('rb') as src:
        for chunk in iter(lambda:src.read(1024*1024),b''): h.update(chunk)
    if h.hexdigest()!=match[1].lower(): raise SystemExit('Checksum mismatch: '+path.name)
    found |= path.name==required
if not found: raise SystemExit('Required payload missing from SHA256SUMS')
PY
}
mapfile -t images < <(find "$base" -maxdepth 1 -type f -name '*.img.xz')
[[ ${#images[@]} == 1 ]]
verify_checksums "$base" "$(basename "${images[0]}")"
base_sha=$(sha256sum "${images[0]}" | cut -d' ' -f1)
image="$output/image/CarLink-refresh-${BASE_RUN_ID}-${REFRESH_COMMIT:0:8}.img"
[[ ! -e "$image" && ! -e "$image.xz" ]]
xz -t -- "${images[0]}"
xz -dc -- "${images[0]}" > "$image"
bash "$project/scripts/verify-image.sh" "$image" --baseline | tee "$output/evidence/base-validation.txt"
loop=$(losetup --find --show --partscan "$image")
udevadm settle
mkdir "$work/root"
mount "${loop}p1" "$work/root"
rootfs="$work/root"
mapfile -t kernels < <(find -L "$rootfs/lib/modules" -mindepth 1 -maxdepth 1 -type d -printf '%f\n')
[[ ${#kernels[@]} == 1 ]]
kernel=${kernels[0]}
start=$(lsblk -nro START "${loop}p1")
boot_prefix=$(dd if="$image" bs=512 count="$start" status=none | sha256sum | cut -d' ' -f1)
app_before=$(sha256sum "$rootfs/usr/local/bin/carlink" | cut -d' ' -f1)
cp "$rootfs/etc/carlink-build.json" "$work/base-build.json"
boot_fingerprint() {
    (cd "$rootfs/boot"; find . -type f -print0 | sort -z | xargs -0 sha256sum; find . -type l -printf '%p -> %l\n' | sort) | sha256sum | cut -d' ' -f1
}
boot_before=$(boot_fingerprint)
profile="$rootfs/etc/NetworkManager/system-connections/carlink-direct.nmconnection"
grep -qx 'interface-name=eth0\|interface-name=end0' "$profile"
sed -i 's/^interface-name=eth0$/interface-name=end0/' "$profile"
chmod 0600 "$profile"
grep -qx 'interface-name=end0' "$profile"

driver_commit=''
printf '[]\n' > "$work/runtime-packages.json"
if [[ -n ${DRIVER_RUN_ID:-} ]]; then
    verify_checksums "$driver_dir" driver.tar.gz
    unpack_driver "$driver_dir/driver.tar.gz" "$work/driver"
    cmp "$driver_dir/driver-build.json" "$work/driver/driver-build.json"
    driver_kernel=$("$python" -c 'import json,sys;print(json.load(open(sys.argv[1]))["kernel_release"])' "$work/driver/driver-build.json")
    variant=$("$python" -c 'import json,sys;print(json.load(open(sys.argv[1]))["variant"])' "$work/driver/driver-build.json")
    driver_commit=$("$python" -c 'import json,sys;print(json.load(open(sys.argv[1]))["driver_commit"])' "$work/driver/driver-build.json")
    [[ "$driver_kernel" == "$kernel" && "$variant" == "$DRIVER_VARIANT" ]]
    # The proven baseline predates the AIC adapter and lacks usb_modeswitch.
    # Downloaded packages are authenticated by Debian APT in the workflow. Extract
    # ARM payloads without invoking maintainer scripts or changing the base libc.
    runtime=${RUNTIME_PACKAGES_DIR:?Mode-switch runtime package directory required}
    "$python" - "$runtime" "$work/runtime-packages.json" <<'PY'
import hashlib,json,pathlib,re,subprocess,sys
directory=pathlib.Path(sys.argv[1]); records=[]; expected={'usb-modeswitch','libjim0.83','usb-modeswitch-data'}
checks={}
for line in (directory/'SHA256SUMS').read_text().splitlines():
    m=re.fullmatch(r'([0-9a-f]{64}) [ *]([^/\\]+\.deb)',line)
    if not m or m[2] in checks: raise SystemExit('Invalid runtime checksum metadata')
    checks[m[2]]=m[1]
for package in sorted(directory.glob('*.deb')):
    digest=hashlib.sha256(package.read_bytes()).hexdigest()
    if checks.get(package.name)!=digest: raise SystemExit('Runtime package checksum mismatch')
    fields=[subprocess.check_output(['dpkg-deb','-f',str(package),name],text=True).strip() for name in ['Package','Version','Architecture']]
    name,version,arch=fields
    if name not in expected or arch!=('all' if name=='usb-modeswitch-data' else 'armhf'):
        raise SystemExit('Unexpected runtime package or architecture: '+repr(fields))
    expected.remove(name)
    records.append({'package':name,'version':version,'architecture':arch,'filename':package.name,'sha256':digest,
                    'installation':'payload extraction only; no foreign maintainer scripts'})
if expected or len(records)!=len(checks): raise SystemExit('Missing or duplicate mode-switch packages')
pathlib.Path(sys.argv[2]).write_text(json.dumps(records,indent=2)+'\n')
PY
    for deb in "$runtime"/*.deb; do dpkg-deb -x "$deb" "$rootfs"; done
    [[ -x "$rootfs/usr/sbin/usb_modeswitch" ]]
    qemu-arm -cpu cortex-a7 -L "$rootfs" "$rootfs/usr/sbin/usb_modeswitch" --version \
        > "$output/evidence/usb-modeswitch-runtime.txt" 2>&1
    while IFS= read -r -d '' module; do
        vermagic=$(modinfo -F vermagic "$module")
        [[ ${vermagic%% *} == "$kernel" ]]
        readelf -h "$module" | grep -q 'Machine:.*ARM'
        printf '%s: %s\n' "${module#"$work/driver/"}" "$vermagic" >> "$output/evidence/driver-vermagic.txt"
    done < <(find "$work/driver/modules" -type f -name '*.ko' -print0)
    cp -a "$work/driver/modules/." "$rootfs/lib/modules/"
    cp -a "$work/driver/firmware/." "$rootfs/lib/firmware/"
    mkdir -p "$rootfs/etc/udev/rules.d"
    install -m 0644 "$work/driver/udev/90-carlink-aic8800.rules" "$rootfs/etc/udev/rules.d/"
    mkdir -p "$rootfs/etc/modprobe.d"
    install -m 0644 "$project/firmware/userpatches/overlay/carlink-aic.conf" "$rootfs/etc/modprobe.d/carlink-aic.conf"
    cmp "$project/firmware/userpatches/overlay/carlink-aic.conf" "$rootfs/etc/modprobe.d/carlink-aic.conf"
    grep -qx 'options aic8800_fdrv country_code=CN custregd=0' "$rootfs/etc/modprobe.d/carlink-aic.conf"
    if [[ "$DISABLE_ONBOARD_WIFI" == true ]]; then
        install -m 0644 "$project/firmware/userpatches/overlay/carlink-onboard-wifi.conf" "$rootfs/etc/modprobe.d/carlink-onboard-wifi.conf"
        cmp "$project/firmware/userpatches/overlay/carlink-onboard-wifi.conf" "$rootfs/etc/modprobe.d/carlink-onboard-wifi.conf"
        grep -Eq '^blacklist[[:space:]]+xradio_wlan([[:space:]]|$)' "$rootfs/etc/modprobe.d/carlink-onboard-wifi.conf"
    fi
    depmod -b "$rootfs" "$kernel"
    # Verify installed content against every declared payload, not just one glob match.
    "$python" - "$work/driver" "$rootfs" <<'PY'
import hashlib,json,pathlib,sys
source,root=map(pathlib.Path,sys.argv[1:]); m=json.loads((source/'driver-build.json').read_text())
for entry in m['modules']+m['firmware']:
    relative=pathlib.PurePosixPath(entry); target=root/'lib'/relative
    if not target.is_file() or target.stat().st_size==0: raise SystemExit('Installed payload missing: '+entry)
    if hashlib.sha256(target.read_bytes()).digest()!=hashlib.sha256((source/relative).read_bytes()).digest():
        raise SystemExit('Installed payload differs: '+entry)
PY
    mkdir -p "$output/driver"
    cp "$driver_dir/driver.tar.gz" "$driver_dir/driver-build.json" "$driver_dir/SHA256SUMS" "$output/driver/"
    cp -a "$runtime" "$output/driver/runtime-packages"
    cp "$work/runtime-packages.json" "$output/driver/runtime-packages/manifest.json"
fi

"$python" "$project/scripts/install-projection-runtime.py" \
    "${APPLICATION_DIR:?Verified application artifact required}" \
    "${CARPLAY_DIR:?Verified CarPlay artifact required}" "$rootfs"
"$python" "$project/scripts/install-boot-profile.py" --root "$rootfs"
[[ $(boot_fingerprint) == "$boot_before" ]]
BASE_SHA="$base_sha" BASE_APP_SHA="$app_before" BOOT_SHA="$boot_before" KERNEL_RELEASE="$kernel" \
DRIVER_COMMIT="$driver_commit" "$python" - "$rootfs" "$output/image/carlink-refresh.json" "$work/runtime-packages.json" "$work/base-build.json" <<'PY'
import datetime,json,os,pathlib,sys
root=pathlib.Path(sys.argv[1]); base=json.loads(pathlib.Path(sys.argv[4]).read_text())
record={'schema_version':1,'refreshed_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
 'base_run_id':os.environ['BASE_RUN_ID'],'base_repository':os.environ.get('SOURCE_REPOSITORY',''),
 'base_image_sha256':os.environ['BASE_SHA'],'base_build':base,
 'refresh_commit':os.environ['REFRESH_COMMIT'],'kernel_release':os.environ['KERNEL_RELEASE'],
 'previous_app_sha256':os.environ['BASE_APP_SHA'],'preserved_boot_tree_sha256':os.environ['BOOT_SHA'],
 'app_run_id':os.environ['APP_RUN_ID'],'app_source_commit':os.environ['APP_COMMIT'],
 'app_sha256':__import__('hashlib').sha256((root/'usr/local/bin/carlink').read_bytes()).hexdigest(),
 'network_interface':'end0','driver_run_id':os.environ.get('DRIVER_RUN_ID') or None,
 'driver_commit':os.environ['DRIVER_COMMIT'] or None,
 'driver_variant':os.environ.get('DRIVER_VARIANT') if os.environ['DRIVER_COMMIT'] else None,
 'runtime_packages':json.loads(pathlib.Path(sys.argv[3]).read_text()),
 'xr819_disabled':os.environ['DISABLE_ONBOARD_WIFI']=='true','hardware_acceptance':'not_tested_by_refresh'}
text=json.dumps(record,indent=2)+'\n'
(root/'etc/carlink-refresh.json').write_text(text)
pathlib.Path(sys.argv[2]).write_text(text)
PY
sync -f "$rootfs"
umount "$rootfs"
losetup -d "$loop"; loop=''
[[ $(dd if="$image" bs=512 count="$start" status=none | sha256sum | cut -d' ' -f1) == "$boot_prefix" ]]
bash "$project/scripts/verify-image.sh" "$image" | tee "$output/image/image-validation.txt"
xz -T2 -3 -- "$image"
xz -t -- "$image.xz"
(cd "$output/image"; sha256sum -- *.img.xz > SHA256SUMS)
echo 'Complete refreshed image ready; original SPL/bootloader/kernel preserved; CI applications installed and image validated.'
