# CarLink native application

Rust binary: **`carlink`**, targeting ARMv7 H3 boards, built with the pinned 1.90 toolchain.
The CarLife MD process is connected to the independent CarPlay receiver; see the [runtime guide](../docs/live-forwarding.md).

## HiCar Rust foundation (R1)

The native application now includes bounded Nearby/DMSDP/HiSight framing,
strict business JSON, RTP/ADTS and reorder tracking, clock conversion, UIBC,
system OpenSSL loading, and a thread-confined HiChain/HUKS FFI adapter.
The diagnostic modes below do not acquire Bluetooth, Wi-Fi, or USB:

```sh
printf '%s\n' '{"op":"uint","value":128}' | carlink hicar codec
python3 scripts/test-hicar-rust.py /path/to/carlink
python3 scripts/test-hicar-rust-auth.py /path/to/carlink /path/to/libcarlink_hichain.so
```

`hicar auth-worker` reads private configuration and protocol messages from
stdin and returns replies over stdout. It is for isolated synthetic tests;
the harness uses temporary stores, logs metadata only, tests both correct and
wrong PINs, and restarts processes to check persisted Rust/Python STS trust.
The C library is loaded on demand; normal CarLife startup does not initialize HUKS.
The production wireless HiCar session remains in Python until R2/R3 acceptance.
See the [migration plan](../docs/hicar-rust.md).

Dependencies are pinned in Cargo.lock. CI packages their source archives and
license notices under `share/source/rust` and `share/licenses/rust`, and includes
them in the firmware checksum checks.

## CarLife transport and synthetic media

```sh
carlink carlife tcp --listen 127.0.0.1:7240 --duration 60
sudo carlink carlife usb --duration 120
```

The application implements framing, MD handshake and video negotiation. Default
mode sends no media. Opt-in synthetic media uses a bounded pre-encoded H.264
loop and quiet intermittent PCM tone; H3 performs no live video encoding.
CarLife 2.0 challenges are answered from the actual transmitted MD_INFO and HU
nonce using the public digest profile. Explicit HU rejection stops the session;
some HUs return acceptance while others proceed directly to video negotiation.
legacy RSA challenges and encrypted media exit with a diagnostic error. The
separate USB peer acceptance notice is sent once after STATISTIC_INFO, independently
of HU_INFO and challenge response; no private identity material is required.

```sh
# Generate assets on the development PC (FFmpeg required), then transfer the
# directory to the board. INIT_DONE advertises the validated asset dimensions.
python scripts/generate-test-media.py .build/test-media-720p --width 1280 --height 720 --fps 30
sudo carlink carlife usb --duration 300 --test-media /opt/carlink/test-media
```

The source validates baseline SPS dimensions/timing, access-unit boundaries and
asset bounds before binding USB. HU dimensions are display preferences; the
actual encoded dimensions from the assets are reported in INIT_DONE. HU fps=0
selects the source rate, capped at 30;
lower requested rates adjust pacing. START begins test playback; PAUSE stops the
test picture and tone, keeping video heartbeats. RESET restarts video from
SPS/PPS+IDR with its mode preserved. HU music STOP remains latched through video
pause/resume. USB media queues are bounded; obsolete queued media is discarded
and dropped video recovers from a keyframe. Timestamps are relative monotonic
milliseconds. These are opt-in development assets; production uses the live CarPlay source.

```sh
# Requires pyusb + libusb-package; keep generated captures out of Git.
python scripts/test-carlife-media.py --usb --output .build/media-capture
ffmpeg -v error -xerror -i .build/media-capture/received.h264 -f null -
```

USB creates a temporary AOA1 FunctionFS gadget, handles requests 51/52/53 and
re-enumerates as `18d1:2d00`. It only owns `carlink-md`, refuses a busy UDC and
cleans up on completion. Use wired SSH during USB tests. The CLI owns only this temporary gadget; normal automatic startup is managed by carlink.service.

TCP is a loopback-only development transport carrying USB-style outer framing,
not the CarLife wireless protocol. Use SSH forwarding for board tests.

```sh
python scripts/test-carlife-lab.py --binary /path/to/carlink
python scripts/test-carlife-lab.py --address 127.0.0.1:17240
```

Update the Actions-built ARMv7 application on the board for app iterations;
no SD reflash is needed. Rust is the main implementation language, Python is
used for independent HU fixtures. Sources and next steps: [roadmap](../docs/roadmap.md).

## Read-only inventory

```sh
carlink doctor --json
```

CI builds `armv7-unknown-linux-gnueabihf` with Ubuntu 24.04's ARM cross linker,
for the selected Debian 13 armhf image. Artifact:
`app/target/armv7-unknown-linux-gnueabihf/release/carlink`. This is a dynamically
linked GNU/Linux binary, not a universal static executable; it requires the ARM
hard-float loader and a compatible glibc on the target. CI records the ELF header.

The JSON schema (version `1`) reports:

- Selected public `/etc/os-release` fields, kernel release, and `MemTotal` in KiB.
- UDC names, state and negotiated speed from `/sys/class/udc`.
- USB topology names, vendor/product IDs, product names and driver symlinks.
- Network interface names, operational state and device driver symlinks.
- `aoa.status: "not_tested"`, always. Presence of a UDC or USB device does not
  establish AOA negotiation, USB bulk transfer, or CarLife compatibility.

Each observation has a `status` and source `path`. Missing files are
`not_present`, access failures are `permission_denied`, malformed memory data is
`invalid_data`, and other I/O errors are `error`. Directory iteration errors yield
`partial` with details. An accessible empty directory is `ok` with no items.
Missing driver links are expected for virtual interfaces such as `lo`; they are
reported explicitly and do not mean the whole board is broken. USB drivers at the
device level may be generic `usb`; the network interface's driver is the useful
Wi-Fi driver identity. All fields are snapshots and devices may disappear while
being read.

The command exits `0` after producing an inventory, including partial/missing
observations; consumers must inspect statuses. Invalid arguments exit `2`.
It reads no Wi-Fi configuration, IP addresses, MAC addresses, USB serial numbers,
passwords or keys. It starts no subprocesses, sends no network traffic and changes
no USB configuration. Root should not be needed for normal sysfs/procfs reads.

Tests cover JSON control escaping, memory units/invalid data, OS field filtering,
missing Linux interfaces and permission error classification. Hardware connectivity
and ARMv7 execution require independent device validation.
