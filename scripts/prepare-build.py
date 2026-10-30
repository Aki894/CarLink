"""Stage the kernel/base image. Formal firmware uses the unified refresh installer."""
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parents[1]
builder = pathlib.Path(sys.argv[1]).resolve()
binary = pathlib.Path(sys.argv[2]).resolve()
carplay = pathlib.Path(sys.argv[3]).resolve()
lock = json.loads((root / "sources.lock.json").read_text())
actual = subprocess.check_output(["git", "-C", str(builder), "rev-parse", "HEAD"], text=True).strip()
if actual != lock["armbian"]["commit"]:
    raise SystemExit("Armbian checkout does not match sources.lock.json")

config = (builder / "config/kernel/linux-sunxi-current.config").read_text()
required = {
    "CONFIG_RTW88_8821CU": "m", "CONFIG_BT_RFCOMM": "m",
    "CONFIG_BT_HCIBTUSB": "m", "CONFIG_USB_CONFIGFS": "m",
    "CONFIG_USB_CONFIGFS_F_FS": "y", "CONFIG_USB_MUSB_SUNXI": "m",
}
for name, value in required.items():
    if f"{name}={value}" not in config.splitlines():
        raise SystemExit(f"Missing kernel prerequisite: {name}={value}")

shutil.copytree(root / "firmware/userpatches", builder / "userpatches", dirs_exist_ok=True)
overlay = builder / "userpatches/overlay"
shutil.copyfile(binary, overlay / "carlink")
if (binary.parent/'share').is_dir():
    shutil.copytree(binary.parent,overlay/'carlink-app',dirs_exist_ok=True)
receiver = json.loads((carplay / "SOURCE.json").read_text())
for key in ["rust", "target", "relay", "stream"]:
    if receiver[key] != lock["carplay"][key]:
        raise SystemExit(f"CarPlay artifact lock mismatch: {key}")
for line in (carplay / "SHA256SUMS").read_text().splitlines():
    expected, name = line.split("  ", 1)
    relative = pathlib.PurePosixPath(name)
    if relative.is_absolute() or ".." in relative.parts:
        raise SystemExit("Unsafe CarPlay checksum path")
    if hashlib.sha256((carplay / name).read_bytes()).hexdigest() != expected:
        raise SystemExit(f"CarPlay artifact checksum mismatch: {name}")
for name in ["bin/openxplay-relay", "bin/oxp-relay", "bin/carlink-audio", "probe.py", "forward.py", "supervisor.py", "run-carplay.sh", "run-carlink.sh",
             "share/admin/index.html", "share/web/receiver/index.html", "share/source/relay.tar.gz",
             "share/source/stream.tar.gz", "share/licenses/relay/LICENSE", "share/licenses/stream/LICENSE"]:
    if not (carplay / name).is_file() or not (carplay / name).stat().st_size:
        raise SystemExit(f"Missing CarPlay runtime payload: {name}")
shutil.copytree(carplay, overlay / "carplay", dirs_exist_ok=True)
shutil.copyfile(root / "scripts/install-boot-profile.py", builder / "userpatches/install-boot-profile.py")
shutil.copytree(root / "firmware/boot-profile", builder / "userpatches/boot-profile", dirs_exist_ok=True)
manifest = {
    "project_commit": subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip(),
    "sources": lock,
    "app_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    "app_notices": (binary.parent/'share').is_dir(),
    "carplay_receiver": receiver,
    "runtime_profile": "baseline",
    "stage": "Kernel/base image only; complete CarLink runtime is installed by refresh",
    "reproducibility": "Framework pinned; resolved kernel/bootloader revisions and package versions remain build-time inputs. See Armbian build logs.",
}
(overlay / "carlink-build.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps(manifest, indent=2))
