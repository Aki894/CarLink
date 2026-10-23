# AIC8800D80 ARMv7 build

Builds the observed `a69c:8d80` adapter against the exact kernel headers from
successful firmware run **39459953269**. The earlier storage ID `a69c:5721`
does not identify its revision. `sources.json` pins both driver profiles and
the actual headers, kernel configuration and Module.symvers hashes.

- `main`: source commit `5327c777d00a94ee3bea09525b3acc8e1b031993`.
- `mcu1`: matched legacy loader/firmware commit
  `a9683a456079131a0381ed75fb7a4f5514527b3c`; retained as a matched alternative. The observed board reports `chip_id=7, chip_mcu_id=1` and has passed with `main`; keep that tested selection rather than switching from the ID alone. Do not mix firmware between profiles.

The workflow downloads that run's artifact, verifies its run succeeded,
verifies the exact headers hashes, builds x86 host tools from the supplied
headers sources, and cross-compiles modules with Debian Trixie GCC 14.
It does not install headers or compile on the board. Missing headers or
Module.symvers are fatal; a generic kernel source is not a substitute.

Artifacts are named `aic8800-usb-main` and `aic8800-usb-mcu1`, containing
`driver.tar.gz`, its `SHA256SUMS`, and `driver-build.json`. Extract the tar
into an empty staging directory. Its root contains `modules/<release>/`,
`firmware/`, `udev/`, `licenses/`, `install.sh`, a manifest and a file checksum
list. The installer validates the running kernel and refuses a profile
switch with AIC modules still loaded. It only copies listed AIC files and
reloads udev; it does not start networking or disable XR819. Preserve Ethernet
SSH as the maintenance path.

The source is fetched into ignored `.build/` at CI time; third-party source
and firmware binaries are not committed to this repository. The runtime
archive includes the source GPL license and any supplied firmware licensing
metadata; the manifest records that the binaries originate from that pinned
upstream tree. Source is available at the recorded repository and commit.

Bluetooth uses the standard kernel `btusb`, with the source's device-scoped
`aic_zlp_quirk` for `368b:8d81`. No replacement `btusb.ko`, custom `aic_btusb`,
global blacklist, `new_id` override or RF calibration changes are applied.
Compiling these modules does not prove firmware upload, Wi-Fi, Bluetooth,
5 GHz AP, radio coexistence, or cold-plug reliability. Validate all of them
before choosing a runtime profile for an image refresh or disabling XR819.

Local validation can inspect a downloaded evidence directory without a
compiler: `bash scripts/build-aic-driver.sh --inspect /path/to/evidence`.
The full build is Linux/Actions-only:
`bash scripts/build-aic-driver.sh main /path/to/evidence dist/aic/main`.
