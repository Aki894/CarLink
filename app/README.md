# CarLink native diagnostic app

Dependency-free Rust binary: **`carlink`**. The first command is a read-only Linux
inventory, intended for ARMv7 H3 boards. It does not implement CarPlay or CarLife yet.

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
