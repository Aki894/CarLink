#!/bin/bash
# Install one CI-built variant. Keep Ethernet and XR819 available throughout.
set -euo pipefail
cd "$(dirname "$0")"
[[ $EUID -eq 0 ]] || { echo 'Run as root'; exit 1; }
sha256sum --check SHA256SUMS
release=$(python3 -c 'import json; print(json.load(open("driver-build.json"))["kernel_release"])')
[[ $(uname -r) == "$release" ]] || { echo 'Running kernel does not match this package'; exit 1; }
command -v usb_modeswitch >/dev/null
[[ -x /usr/sbin/usb_modeswitch ]] || { echo 'Expected /usr/sbin/usb_modeswitch (Debian usb-modeswitch package)'; exit 1; }
# Refuse an in-place switch while a previous variant remains resident. Copying
# new firmware underneath an active loader can mix otherwise matched variants.
if lsmod | awk '{print $1}' | grep -Eq '^(aic_load_fw|aic8800_fdrv|aic_zlp_quirk|aic_btusb)$'; then
    echo 'Unplug the USB adapter, unload the AIC modules, then rerun this installer.'
    exit 1
fi
python3 - <<'PY'
import json, pathlib, shutil
root = pathlib.Path.cwd()
m = json.loads((root / 'driver-build.json').read_text())
release = m['kernel_release']
groups = [('modules', pathlib.Path('/lib/modules')), ('firmware', pathlib.Path('/lib/firmware')),
          ('udev', pathlib.Path('/etc/udev/rules.d'))]
for group, target in groups:
    for name in m[group]:
        relative = pathlib.PurePosixPath(name)
        if relative.is_absolute() or '..' in relative.parts or relative.parts[0] != group:
            raise SystemExit('Unsafe manifest member: ' + name)
        source = root / name
        if source.is_symlink() or not source.is_file():
            raise SystemExit('Missing or linked package member: ' + name)
        destination = target.joinpath(*relative.parts[1:])
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        destination.chmod(0o644)
pathlib.Path('/var/lib/carlink').mkdir(parents=True, exist_ok=True)
shutil.copyfile(root / 'driver-build.json', '/var/lib/carlink/aic-driver-build.json')
PY
depmod -a "$release"
udevadm control --reload-rules
echo 'Installed. Replug the adapter, then inspect journalctl -k, lsusb -t, iw list and btmgmt info.'
echo 'No radio, network connection, Bluetooth service, XR819 setting or existing btusb module was changed.'
