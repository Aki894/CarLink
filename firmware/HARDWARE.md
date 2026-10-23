# WuKongPi Hi-Zero H3 hardware baseline

Target: the user's WuKongPi Hi-Zero / H3 Zero, 512 MiB, XR819 onboard Wi-Fi,
AX900/AIC8800D80 Wi-Fi/Bluetooth module on the board's USB-A connector, Ethernet
connected directly to the development PC. Hardware, USB and phone bench checks are recorded in [validation](../docs/validation.md). XR819 was disabled after AIC Wi-Fi, Bluetooth and AP acceptance.

## Evidence and local documents

The local documents were inspected without unpacking the large images:

- `WuKong Pi Hi-Zero/悟空派H3 Zero硬件手册.pdf`:
  PDF pages 3–4 identify H3, 256/512 MB DDR3, onboard 100 Mb Ethernet,
  three independent USB Host ports including two on the expansion header.
- `WuKong Pi Hi-Zero/Wukong pi H3 用户手册_v3.2.pdf`:
  PDF pages 3–4 identify the Type-C power connection, 13-pin expansion
  connector, and the vendor SDK repository.
- `WuKong Pi Hi-Zero/SCH/WuKongPi-H3-zero-SCH.pdf`:
  five raster schematic sheets, visually inspected; sheet 1 is DDR3,
  sheet 2 CPU/pins, sheet 3 power/Type-C, sheet 4 USB/SD/Ethernet,
  sheet 5 XR819. The schematic has a generic DDR3 footprint description,
  not proof of the memory part fitted to this individual board.
- `WuKong Pi Hi-Zero/WuKong Pi H3 Zero User Manual.pdf`:
  PDF pages 3–4 describe Android 4.4 and separate USB Host/OTG, but also
  contain an H2/H3 inconsistency. Prefer the Chinese hardware manual and
  schematic for board identification.

Vendor SDK pinned for evidence:
[`Timfu2019/wukongpi_build@d2506854fd12fe31009af3d01a6122a583be229e`](https://github.com/Timfu2019/wukongpi_build/tree/d2506854fd12fe31009af3d01a6122a583be229e).
Its [board configuration](https://github.com/Timfu2019/wukongpi_build/blob/d2506854fd12fe31009af3d01a6122a583be229e/config/boards/wukongpi.conf)
and [U-Boot DTS patch](https://github.com/Timfu2019/wukongpi_build/blob/d2506854fd12fe31009af3d01a6122a583be229e/patch/u-boot/u-boot-sunxi/board_wukongpi/add-wukongpai-dts.patch)
use the Orange Pi Zero topology with WuKongPi names. This supports using
`BOARD=orangepizero` as the modern build baseline, rather than an H616/H618
Zero 2/3 target. It does not establish that an unmodified image works.

## Memory and bootloader

The [vendor U-Boot config patch](https://github.com/Timfu2019/wukongpi_build/blob/d2506854fd12fe31009af3d01a6122a583be229e/patch/u-boot/u-boot-sunxi/board_wukongpi/add-wukongpai-cfg.patch)
sets `DRAM_CLK=408`, `DRAM_ZQ=3881979`, and `DRAM_ODT_EN=y`.
Upstream [U-Boot v2026.07 Orange Pi Zero configuration](https://github.com/u-boot/u-boot/blob/v2026.07/configs/orangepi_zero_defconfig)
uses `DRAM_CLK=624`. However, the pinned Armbian framework applies its own
board-specific patch first: it already sets DRAM to 408 MHz, ZQ to 3881979,
enables ODT and sets the boot CPU to 480 MHz, matching the vendor baseline.
A second patch against the unmodified upstream file would conflict with it.
This project therefore does not duplicate that patch.

The old vendor configuration also used `SYS_CLK_FREQ=480000000`; current
H3 U-Boot defaults to 1008000000. Armbian's board patch selects the vendor's
480 MHz bootloader CPU setting. Linux CPU frequency policy is separate.
The board reports MemTotal=501244 KiB, consistent with the 512 MiB model. Repeated cold boots and memory stress remain follow-up checks before any DRAM clock increase.

Armbian is pinned to
[`5eda7293bab43a581de02f769139848b6e922328`](https://github.com/armbian/build/tree/5eda7293bab43a581de02f769139848b6e922328).
Its [sunxi include](https://github.com/armbian/build/blob/5eda7293bab43a581de02f769139848b6e922328/config/sources/families/include/sunxi_common.inc)
selects `BOOTBRANCH=tag:v2026.07` and **`BOOTPATCHDIR=v2026.07-sunxi`**.
The framework's [board patch](https://github.com/armbian/build/blob/5eda7293bab43a581de02f769139848b6e922328/patch/u-boot/v2026.07-sunxi/board_orangepizero/0001-orangepi-zero-dram-cpufreq.patch)
supplies these settings. Our `uboot_make_config` extension checks all four
final values after `olddefconfig` and before compilation. A mismatch stops
the build; the actual `.config` is preserved as `carlink-uboot.config` in
the image artifact. The first baseline disables artifact reuse so this
gate runs during compilation.

## DTS and connectors

| Function | Schematic / vendor DTS | Baseline requirement |
| --- | --- | --- |
| USB Type-C | USB0 D+/D−; CC1/CC2 have 5.1 kOhm pull-downs; PG12/USB0-ID has a 10 kOhm pull-up | Keep USB0 in `peripheral` mode for the car-facing gadget; the vendor DTS already matches PG12 |
| USB-A | USB1, 5 V VBUS from the board rail | Enable EHCI/OHCI1 for AX900/AIC8800D80 independently of USB0 |
| Expansion USB | 13-pin pins 3/4 are USB2 D−/D+; pins 5/6 USB3 D−/D+; pins 1/2 are 5 V/GND | `usbhost2`/`usbhost3` overlays are only needed for these extra connectors |
| Ethernet | H3 internal EPHY, magnetics and RJ45; no external PHY IC | `emac` enabled, `phy-mode="mii"`, `phy-handle=<&int_mii_phy>` |
| SD | SD0, card detect PF6 | `mmc0` 4-bit with PF6 active-low card detect |
| CPU supply | PL6 switches the regulator's voltage selection | Retain the existing 1.1/1.3 V CPU regulator mapping |
| XR819 | SDIO1; PA20 power-enable and PL7 reset | Vendor mapping agrees with Orange Pi Zero; keep separate from AX900/AIC8800D80 selection |
| Debug UART | UART0 on dedicated three-pin connector | 3.3 V TTL, 115200 8N1; never use RS-232 voltage levels |

The vendor DTS and current Orange Pi Zero DTS both set USB0
`dr_mode="peripheral"`; no speculative change to `otg` is necessary.
USB Host1 can operate while USB0 is a device, so AX900/AIC8800D80 does not consume
the car-facing gadget port. The Type-C VBUS and USB-A supply share the
board's 5 V rail in the schematic: do not assume source isolation or
backfeed protection when planning external power injection.

For AOA development, an automatically loaded `g_serial` occupies the UDC
and must be disabled or explicitly unbound before the AOA implementation
claims it. Network/SSH diagnostics remain available over the direct
Ethernet connection; this does not require gadget Ethernet or a USB hub.

## Hardware follow-up

Ethernet, simultaneous USB Host/Device, AOA, AIC Wi-Fi/Bluetooth/AP and software reboot recovery passed the recorded bench checks. Remaining work is repeated cold starts, memory/SD stress and Lexus vehicle acceptance; see [validation](../docs/validation.md).
