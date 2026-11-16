# CarLink licensing overview

The MIT License applies only to original CarLink material for which the CarLink contributors hold the necessary rights. It does not apply to OpenXPlay-Relay, OpenXPlay-Stream, their bundled modified versions or modifications to them; these remain under AGPL-3.0-only and the upstream notices and additional terms. DiPlay/xcertplay-derived portions remain under GPL-3.0-only. Other third-party components remain under their own licenses.

| Component / scope | Copyright holders | License |
| --- | --- | --- |
| Original CarLink application, integration scripts and documentation | Copyright (c) 2026 CarLink contributors | [MIT](LICENSES/MIT.txt) |
| [OpenXPlay-Relay](https://github.com/OpenXPlay/OpenXPlay-Relay), modified builds and patches outside the derived files below | Copyright (c) 2026 OpenXPlay.org and contributors; CarLink contributors for their modifications | [AGPL-3.0-only](LICENSES/AGPL-3.0-only.txt), with [upstream Section 7 additional terms](LICENSES/OpenXPlay-Relay-NOTICE.txt) |
| [OpenXPlay-Stream](https://github.com/OpenXPlay/OpenXPlay-Stream) and bundled builds | Copyright (c) 2026 OpenXPlay.org and contributors | AGPL-3.0-only, with [upstream Section 7 additional terms](LICENSES/OpenXPlay-Stream-NOTICE.txt) |
| Relay `rust/carplay-iap2/`, `rust/carplay-airplay/`, and `rust/direlay-linux/src/network/{air_play_port_selector,hotspot_address_policy,wireless_host_address,wireless_hotspot_manager}.rs`, including corresponding patches | DiPlay/xcertplay authors and contributors; OpenXPlay and CarLink contributors for their modifications; individual file headers retained | [GPL-3.0-only](LICENSES/GPL-3.0-only.txt) |
| FFmpeg | FFmpeg authors and contributors | LGPL-2.1-or-later (configured build) |
| Opus | Xiph.Org Foundation and contributing authors | BSD-3-Clause |
| minih264 / stb / Noto CJK fonts | Their respective authors; see bundled notices | MIT / MIT or public domain / OFL-1.1 |
| Linux, Armbian, wireless drivers, system packages and other dependencies | Their respective authors; see package and upstream notices | Their own licenses, including GPL; the firmware is a multi-license distribution |

Powered by OpenXPlay. Visit [OpenXPlay](https://openxplay.org/). Preserve the full Relay and Stream notices, including the DiPlay (https://github.com/shihabal3amri/DiPlay) and xcertplay (https://github.com/shilapi/xcertplay) attribution. The network source requirement applies to the combined GPL/AGPL receiver as described in the upstream NOTICE.

`sources.lock.json` pins the upstream revisions. [Patch modification records](receiver/patches/MODIFICATIONS.md) identify the changed upstream files and dates. Original file copyright and license headers must be retained. Bundled dependency notices and sources accompany the firmware under `/usr/local/lib/carlink/`; consult the release's source archive and build metadata for the exact distributed version.
