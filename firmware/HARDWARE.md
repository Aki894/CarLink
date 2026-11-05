# Hardware requirements

The current firmware build targets an Allwinner H3 / ARMv7 platform using the Orange Pi Zero-compatible board configuration. Verify the board's boot configuration, memory settings and USB wiring before using the image on another H3 board.

- **USB peripheral port with a Linux UDC** for the CarLife link. Connect this data port to the head unit.
- **Separate USB host port** for the external wireless adapter. The firmware bundles support for the AIC8800D80 family. Check the chipset and interface rather than relying on a retail adapter name.
- **Wi-Fi access point and Bluetooth** supported concurrently by the adapter and drivers.
- **Ethernet** for setup and maintenance. Keep it available when changing wireless or USB configuration.

`carlink doctor --json` reports USB and platform information. `carlink wifi-interface` shows the detected Wi-Fi interface. Verify the external adapter before disabling onboard wireless. See [installation](../docs/provisioning.md).
