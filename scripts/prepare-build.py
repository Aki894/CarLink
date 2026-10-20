"""Stage the pinned Armbian tree and a previously built ARMv7 application."""
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parents[1]
builder = pathlib.Path(sys.argv[1]).resolve()
binary = pathlib.Path(sys.argv[2]).resolve()
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
manifest = {
    "project_commit": subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip(),
    "sources": lock,
    "app_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    "stage": "hardware bring-up; CarPlay and CarLife not implemented",
    "reproducibility": "Framework pinned; resolved kernel/bootloader revisions and package versions remain build-time inputs. See Armbian build logs.",
}
(overlay / "carlink-build.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps(manifest, indent=2))
