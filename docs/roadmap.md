# Linux bring-up roadmap

1. Build a pinned Armbian/Orange Pi Zero compatible Debian 13 minimal image.
   Apply the vendor's conservative H3 DRAM setting. Preserve Ethernet SSH, logs,
   independent USB Host and peripheral-mode OTG.
2. AX900/AIC8800D80 Wi-Fi, Bluetooth discovery, 5 GHz AP and reboot recovery
   passed the [board acceptance](board-acceptance-2026-10-21.md). XR819 is disabled.
   Memory stress and cold-start timing remain later tasks.
3. Implement and independently test Linux CarLife MD transport. Exercise AOA
   requests 51/52/53, re-enumeration, bulk transfers, fragmentation, disconnects.
   First use generated H.264 and PCM, then validate the actual Lexus head unit.
4. Integrate the wireless CarPlay receiver after CarLife MD works on the Lexus.
   Use CatPlay's Rust architecture and DiPlay as references. Confirm upstream
   licensing and the local MFi identity/hardware path before importing components.
5. Integrate bounded media queues, timestamps, audio roles, H.264 recovery and
   Remote Touch/HID. Validate navigation, reconnects, microphone/calls separately.
6. Optimize cold startup only after both protocol endpoints work reliably.

Current code adds the first Rust CarLife MD milestone: bounded wire framing,
handshake/video negotiation, loopback TCP lab and temporary FunctionFS AOA1 USB.
Video/audio producers and real Lexus acceptance are the next milestone.

Implementation choice: Rust for the headless application, Python for host-side
test fixtures. Keep app deployment independent of kernel/image builds.

Local reference snapshots (protocol facts and architecture, not bundled sources):

- `Carlife_on_box` `d9397d90a1f32670f034eb07cdbe28d887a179d8`: native MD protocol and session handling.
- `headswap` `99e888cf546ec5f040d4de377c59b478186afd5b`: protocol framing, version handshake and independent HU tools.
- `catplay` `bc6edae49554f2d0160b18ed18fc7b7376353115`: Rust CarPlay receiver architecture; integration follows CarLife.
- [AOA 1.0 specification](https://source.android.com/docs/core/interaction/accessories/aoa).
- [Linux FunctionFS documentation](https://docs.kernel.org/usb/functionfs.html).

Armbian support status and configuration:

- [Orange Pi Zero community board](https://github.com/armbian/build/blob/5eda7293bab43a581de02f769139848b6e922328/config/boards/orangepizero.csc)
- [ARM sunxi kernel/bootloader selection](https://github.com/armbian/build/blob/5eda7293bab43a581de02f769139848b6e922328/config/sources/families/include/sunxi_common.inc)
- [Native driver and gadget config](https://github.com/armbian/build/blob/5eda7293bab43a581de02f769139848b6e922328/config/kernel/linux-sunxi-current.config)

The framework commit is fixed, but source branch heads and apt repositories may
advance. Preserve each build's resolved versions and logs. Do not describe the
build as bit-for-bit reproducible without also pinning those inputs.
