# CarLink modifications to OpenXPlay-Relay and OpenXPlay-Stream

Modified by CarLink contributors. Dates below are the last recorded CarLink commit date for each patch. Each named file is modified by that patch; the patch itself records the exact changes. Upstream Relay revision: `6a7cdab585113e29422f2306e90fdb5a789c318a`; Stream revision: `356a3f56c0e0b4e839b91387bf2b8cfa0b0a8537`. Patch names ending in `-stream.patch` apply to Stream.

| Patch | Modification date | Upstream file | License |
| --- | --- | --- | --- |
| 0001-buffered-audio-diagnostics.patch | 2026-10-22 | `rust/carplay-airplay/src/air_play_session.rs` | GPL-3.0-only |
| 0001-buffered-audio-diagnostics.patch | 2026-10-22 | `rust/direlay-linux/src/linux_audio.rs` | AGPL-3.0-only + NOTICE |
| 0001-buffered-audio-diagnostics.patch | 2026-10-22 | `rust/direlay-linux/src/linux_wireless_platform.rs` | AGPL-3.0-only + NOTICE |
| 0002-advertise-buffered-and-pcm-audio.patch | 2026-10-22 | `rust/carplay-airplay/src/air_play_info_plist.rs` | GPL-3.0-only |
| 0002-advertise-buffered-and-pcm-audio.patch | 2026-10-22 | `rust/carplay-airplay/tests/control_info.rs` | GPL-3.0-only |
| 0002-advertise-buffered-and-pcm-audio.patch | 2026-10-22 | `rust/direlay-linux/src/linux_audio.rs` | AGPL-3.0-only + NOTICE |
| 0003-telephony-voice-rates.patch | 2026-10-22 | `rust/carplay-airplay/src/air_play_info_plist.rs` | GPL-3.0-only |
| 0003-telephony-voice-rates.patch | 2026-10-22 | `rust/carplay-airplay/tests/control_info.rs` | GPL-3.0-only |
| 0004-opt-in-opus-voice.patch | 2026-10-26 | `rust/direlay-linux/src/linux_wireless_platform.rs` | AGPL-3.0-only + NOTICE |
| 0005-live-video-deadlines.patch | 2026-10-22 | `rust/direlay-linux/src/stream_bridge_sink.rs` | AGPL-3.0-only + NOTICE |
| 0005-live-video-deadlines.patch | 2026-10-22 | `rust/direlay-linux/src/stream_connection.rs` | AGPL-3.0-only + NOTICE |
| 0006-headless-30fps.patch | 2026-10-22 | `rust/direlay-linux/src/linux_wireless_platform.rs` | AGPL-3.0-only + NOTICE |
| 0007-hide-inert-oem-tile.patch | 2026-10-23 | `rust/direlay-linux/src/linux_wireless_platform.rs` | AGPL-3.0-only + NOTICE |
| 0008-explicitly-hide-cached-oem-tile.patch | 2026-10-23 | `rust/carplay-airplay/src/air_play_info_plist.rs` | GPL-3.0-only |
| 0008-explicitly-hide-cached-oem-tile.patch | 2026-10-23 | `rust/carplay-airplay/tests/control_info.rs` | GPL-3.0-only |
| 0008-explicitly-hide-cached-oem-tile.patch | 2026-10-23 | `rust/direlay-linux/src/linux_wireless_platform.rs` | AGPL-3.0-only + NOTICE |
| 0009-carlink-connection-names.patch | 2026-10-25 | `rust/direlay-linux/src/branding.rs` | AGPL-3.0-only + NOTICE |
| 0009-carlink-connection-names.patch | 2026-10-25 | `rust/direlay-linux/src/linux_hotspot.rs` | AGPL-3.0-only + NOTICE |
| 0009-carlink-connection-names.patch | 2026-10-25 | `rust/direlay-linux/src/provision.rs` | AGPL-3.0-only + NOTICE |
| 0010-new-device-only-pairing.patch | 2026-10-25 | `rust/direlay-linux/src/host_backends.rs` | AGPL-3.0-only + NOTICE |
| 0010-new-device-only-pairing.patch | 2026-10-25 | `rust/direlay-linux/src/linux_bluetooth.rs` | AGPL-3.0-only + NOTICE |
| 0010-new-device-only-pairing.patch | 2026-10-25 | `rust/direlay-linux/src/linux_hotspot.rs` | AGPL-3.0-only + NOTICE |
| 0010-new-device-only-pairing.patch | 2026-10-25 | `rust/direlay-linux/src/linux_wireless_platform.rs` | AGPL-3.0-only + NOTICE |
| 0010-new-device-only-pairing.patch | 2026-10-25 | `rust/direlay-linux/src/main.rs` | AGPL-3.0-only + NOTICE |
| 0011-production-vehicle-keys.patch | 2026-10-28 | `rust/carplay-airplay/src/air_play_session.rs` | GPL-3.0-only |
| 0011-production-vehicle-keys.patch | 2026-10-28 | `rust/carplay-airplay/src/car_play_media_engine.rs` | GPL-3.0-only |
| 0011-production-vehicle-keys.patch | 2026-10-28 | `rust/carplay-airplay/src/media_sink_decorator.rs` | GPL-3.0-only |
| 0011-production-vehicle-keys.patch | 2026-10-28 | `rust/direlay-linux/src/di_play_input.rs` | AGPL-3.0-only + NOTICE |
| 0011-production-vehicle-keys.patch | 2026-10-28 | `rust/direlay-linux/src/linux_air_play_server.rs` | AGPL-3.0-only + NOTICE |
| 0012-control-peer-liveness.patch | 2026-10-28 | `rust/direlay-linux/src/linux_air_play_server.rs` | AGPL-3.0-only + NOTICE |
| 0013-functional-carlink-oem-menu.patch | 2026-10-29 | `rust/direlay-linux/src/branding.rs` | AGPL-3.0-only + NOTICE |
| 0013-functional-carlink-oem-menu.patch | 2026-10-29 | `rust/direlay-linux/src/di_play_input.rs` | AGPL-3.0-only + NOTICE |
| 0013-functional-carlink-oem-menu.patch | 2026-10-29 | `rust/direlay-linux/src/linux_wireless_platform.rs` | AGPL-3.0-only + NOTICE |
| 0013-functional-carlink-oem-menu.patch | 2026-10-29 | `rust/direlay-linux/src/stream_connection.rs` | AGPL-3.0-only + NOTICE |
| 0014-carlink-host-ui-stream.patch | 2026-10-29 | `rust/oxp-wire/src/lib.rs` | AGPL-3.0-only + NOTICE |
| 0015-bounded-buffered-audio-prefetch.patch | 2026-10-30 | `rust/carplay-airplay/src/buffered_audio_stream.rs` | GPL-3.0-only |
| 0015-bounded-buffered-audio-prefetch.patch | 2026-10-30 | `rust/carplay-airplay/tests/media_workers.rs` | GPL-3.0-only |
| 0016-relative-wheel-focus.patch | 2026-11-01 | `rust/carplay-airplay/src/air_play_session.rs` | GPL-3.0-only |
| 0017-openxplay-attribution-and-source.patch | 2026-11-16 | `Relay: admin-web/src/main.ts` | AGPL-3.0-only + NOTICE |
| 0017-openxplay-attribution-and-source.patch | 2026-11-16 | `Relay: admin-web/src/style.css` | AGPL-3.0-only + NOTICE |
| 0017-openxplay-attribution-and-source.patch | 2026-11-16 | `Relay: rust/direlay-linux/src/admin/server.rs` | AGPL-3.0-only + NOTICE |
| 0017-openxplay-attribution-and-source.patch | 2026-11-16 | `Relay: rust/direlay-linux/tests/wp4b_services.rs` | AGPL-3.0-only + NOTICE |
| 0018-corresponding-source-offer-stream.patch | 2026-11-16 | `Stream: rust/web-receiver/receiver/index.html` | AGPL-3.0-only + NOTICE |
