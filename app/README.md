# CarLink native application

Dependency-free Rust binary: **`carlink`**, targeting ARMv7 H3 boards.
CarLife MD development comes first; CarPlay integration follows it.

## CarLife transport and synthetic media

```sh
carlink carlife tcp --listen 127.0.0.1:7240 --duration 60
sudo carlink carlife usb --duration 120
```

The application implements framing, MD handshake and video negotiation. Default
mode sends no media. Opt-in synthetic media uses a bounded pre-encoded H.264
loop and quiet intermittent PCM tone; H3 performs no live video encoding.
Unsupported authentication or encryption requests exit with an error.

```sh
# Generate assets on the development PC (FFmpeg required), then transfer the
# directory to the board. Match the HU's requested video dimensions exactly.
python scripts/generate-test-media.py .build/test-media-720p --width 1280 --height 720 --fps 30
sudo carlink carlife usb --duration 300 --test-media /opt/carlink/test-media
```

The source validates baseline SPS dimensions/timing, access-unit boundaries and
asset bounds before binding USB. HU fps=0 selects the source rate, capped at 30;
lower requested rates adjust pacing. START begins test playback; PAUSE stops the
test picture and tone, keeping video heartbeats. RESET restarts video from
SPS/PPS+IDR with its mode preserved. HU music STOP remains latched through video
pause/resume. USB media queues are bounded; obsolete queued media is discarded
and dropped video recovers from a keyframe. Timestamps are relative monotonic
milliseconds. These are development assets, pending a live CarPlay media source.

```sh
# Requires pyusb + libusb-package; keep generated captures out of Git.
python scripts/test-carlife-media.py --usb --output .build/media-capture
ffmpeg -v error -xerror -i .build/media-capture/received.h264 -f null -
```

USB creates a temporary AOA1 FunctionFS gadget, handles requests 51/52/53 and
re-enumerates as `18d1:2d00`. It only owns `carlink-md`, refuses a busy UDC and
cleans up on completion. Use wired SSH during USB tests. No boot service is installed.

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
cargo test --locked --manifest-path app/Cargo.toml
cargo build --release --locked --manifest-path app/Cargo.toml
./app/target/release/carlink doctor --json
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
