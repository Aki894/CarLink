# Linux CarPlay receiver

CarLink runs the [OpenXPlay Linux relay](https://github.com/OpenXPlay/OpenXPlay-Relay)
as an independent process. `sources.lock.json` pins both Relay and Stream commits.
GitHub Actions builds ARMv7 hard-float executables and web assets. The artifact
includes upstream licenses, source archives, source metadata and checksums.

Powered by OpenXPlay. Relay is AGPL-3.0-only, and its DiPlay-derived protocol
libraries are GPL-3.0-only; the existing CarLink application remains a separate
process. Preserve the upstream notices and attribution when distributing it.

## Run on the board

Install the `carplay-armv7` artifact under `/usr/local/lib/carlink/carplay`, restore
execute permissions on `bin/*`, `probe.py` and `run-carplay.sh`, and install
`run-carplay.sh` as `/usr/local/bin/carlink-carplay`. The firmware build performs
these steps automatically. The combined carlink.service starts the receiver automatically when local identity files are present. Stop that service before running standalone probe/web commands.

The local identity directory is `/var/lib/carlink/carplay/mfi`, containing
`identity.pk8` and `certificate.p7b`. Keep directory mode 0700 and file mode 0600.
Pairing state lives in the adjacent `state` directory. None of these files belongs
in Git, CI artifacts, or firmware. The launcher requires root to manage Bluetooth
and the wireless hotspot; Ethernet management remains connected.

```sh
carlink-carplay probe --pair 600
```

On the iPhone, pair with **OpenXPlay Linux** and allow CarPlay. `probe` replaces
the upstream output subprocess with a Python OXP consumer. It advertises H.264,
reports video/audio metadata and drains media without decoding or displaying it.
`carlink-carplay web --pair 600` uses the original web output subprocess instead.
The optional environment variables `CARLINK_CARPLAY_ROOT`,
`CARLINK_CARPLAY_STATE`, and `CARLINK_CARPLAY_WIFI` override install/state/interface
paths. `CARLINK_CARPLAY_COUNTRY` defaults to `CN` and
`CARLINK_CARPLAY_CHANNEL` to the previously tested non-DFS channel 149. The
default interface is the tested AX900 adapter `wlx002e2d91fc8e`.

Probe status is `/run/carlink/carplay-probe.json`. The producer connects to
`/run/carlink-carplay/media.sock`, and the local control socket appends `.control`.

```sh
/usr/local/lib/carlink/carplay/probe.py \
  --send-control /run/carlink-carplay/media.sock.control
```

The default control is a keyframe request. A touch command uses normalized
coordinates, for example:

```sh
/usr/local/lib/carlink/carplay/probe.py \
  --send-control /run/carlink-carplay/media.sock.control \
  --command '{"type":"touch","contacts":[{"id":0,"x":0.5,"y":0.5,"down":true}]}'
```

Recording is opt-in via `CARLINK_PROBE_RECORD_DIR`. It writes Annex B H.264 from
the next keyframe, with a 128 MiB total limit across existing segments. Captures
may contain personal screen content; keep them local. Without this variable, no
media is saved. Audio is counted and its format reported in probe mode.
`carlink-carplay forward` uses the new live consumer; the combined
`carlink-run` supervisor starts it alongside the CarLife USB sender. See
[live forwarding](../docs/live-forwarding.md) for runtime and acceptance details.

## Wire boundary and checks

OXP v0 uses a four-byte little-endian message length, followed by JSON or a
24-byte binary header and payload. Video uses four-byte big-endian NAL lengths;
the probe converts these to Annex B and converts AVC configuration to SPS/PPS.
The maximum wire message is 16 MiB, JSON is capped at 256 KiB, and local controls
are capped at 4096 bytes. Repeated unchanged video configuration does not trigger
another keyframe request. SIGTERM closes the owned Unix sockets.

Run `python3 -m unittest discover -s receiver` for malformed-frame, AVC/NAL,
touch and supervised-process checks. Linux Actions runs the Unix process test;
the optional upstream vector test uses the ignored local reference checkout.

The existing CarLife synthetic-media path remains independently runnable. The live
consumer forwards H.264, converts media/alert audio and returns single-point
touch. Phone/Siri microphone paths are a later stage.
