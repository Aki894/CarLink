# CarLink

[简体中文](README.zh-CN.md)

**Wireless CarPlay and HiCar for head units with wired Baidu CarLife.**

CarLink runs on an external Linux adapter. It is designed for factory and other non-Android head units that support wired CarLife and cannot install receiver apps such as DiPlay. The head unit continues to run its built-in CarLife interface.

```text
iPhone (CarPlay) or Huawei phone (HiCar)
             │ Bluetooth + Wi-Fi
         CarLink adapter
             │ USB (wired CarLife)
          Head unit
```

- Wireless CarPlay and HiCar, one active protocol at a time.
- Video, music, navigation prompts, call audio and microphone forwarding.
- Touch and supported head-unit controls, with an on-screen device menu and volume adjustment.
- Automatic reconnection to a remembered device, with manual protocol and device switching.

## Requirements

- A head unit with **wired Baidu CarLife**. Controls and audio channels depend on the head unit.
- A compatible Linux adapter with USB peripheral mode, Wi-Fi and Bluetooth. The firmware build targets an Allwinner H3 platform. Check the [hardware requirements](firmware/HARDWARE.md) before flashing.
- An iPhone for wireless CarPlay or a Huawei phone with HiCar on HarmonyOS 4.2. Newer HarmonyOS versions are outside the current target.
- For CarPlay, supply your own compatible MFi identity files. HiCar generates its device identity locally.

## Get started

1. Obtain a complete firmware image and verify its checksum. See [installation](docs/provisioning.md).
2. Flash the adapter, connect it to the head unit's CarLife USB port and open CarLife.
3. In the CarLink menu, select CarPlay or HiCar and choose to connect a device. Follow the phone prompts. For a new HiCar device, use the PIN shown on screen.

See the [documentation index](docs/README.md) for controls, troubleshooting and development. CarLink is an independent project. See [LICENSE](LICENSE) and the [receiver integration notes](receiver/README.md) for upstream components.
