# Linux bring-up roadmap

1. Build a pinned Armbian/Orange Pi Zero compatible Debian 13 minimal image.
   Apply the vendor's conservative H3 DRAM setting. Preserve Ethernet SSH, logs,
   independent USB Host and peripheral-mode OTG.
2. Validate the real board: memory stability, USB topology, RTL8821CU identity,
   native Wi-Fi firmware, USB Bluetooth firmware, legal 5 GHz AP channels.
   Network enumeration is not a connectivity or stability test.
3. Implement and independently test Linux CarLife MD transport. Exercise AOA
   requests 51/52/53, re-enumeration, bulk transfers, fragmentation, disconnects.
   First use generated H.264 and PCM, then validate the actual Lexus head unit.
4. Cross compile and independently validate an OpenXPlay-based CarPlay receiver
   on ARMv7. Valid local accessory identity is required and is never committed.
5. Integrate bounded media queues, timestamps, audio roles, H.264 recovery and
   Remote Touch/HID. Validate navigation, reconnects, microphone/calls separately.
6. Optimize cold startup only after both protocol endpoints work reliably.

Current code implements only read-only inventory (`carlink doctor --json`).
There is no claim of a successful Linux CarLife or CarPlay session yet.

Armbian support status and configuration:

- [Orange Pi Zero community board](https://github.com/armbian/build/blob/5eda7293bab43a581de02f769139848b6e922328/config/boards/orangepizero.csc)
- [ARM sunxi kernel/bootloader selection](https://github.com/armbian/build/blob/5eda7293bab43a581de02f769139848b6e922328/config/sources/families/include/sunxi_common.inc)
- [Native driver and gadget config](https://github.com/armbian/build/blob/5eda7293bab43a581de02f769139848b6e922328/config/kernel/linux-sunxi-current.config)

The framework commit is fixed, but source branch heads and apt repositories may
advance. Preserve each build's resolved versions and logs. Do not describe the
build as bit-for-bit reproducible without also pinning those inputs.
