"""Independent CarLife HU media probe; saves USB payloads for FFmpeg validation.

Requires the opt-in test source with 1280x720 assets. --usb uses the exact lab
serial and AOA helper. Otherwise --address can be an SSH-forwarded TCP port.
"""
import argparse
import importlib.util
import json
import pathlib
import socket
import struct
import time


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), pathlib.Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


lab = load('test-carlife-lab')


def packet(stream):
    channel, length = struct.unpack('>II', lab.exact(stream, 8))
    assert 1 <= channel <= 6 and 8 <= length <= 1024 * 1024 + 12, (channel, length)
    raw = lab.exact(stream, length)
    if channel in (1, 6):
        size, reserved, kind = struct.unpack('>HHI', raw[:8])
        assert reserved == 0 and size == length - 8
        return channel, kind, None, raw[8:]
    size, stamp, kind = struct.unpack('>III', raw[:12])
    assert size == length - 12
    return channel, kind, stamp, raw[12:]


def probe(stream, output):
    stream.settimeout(3)
    stream.sendall(lab.command(0x18001, b'\x08\x02\x10\x00'))
    assert [packet(stream)[1] for _ in range(4)] == [0x10002, 0x1001b, 0x10018, 0x10004]
    # Actual Java HU behavior: fps=0 means unrestricted.
    stream.sendall(lab.command(0x18007, b'\x08\x80\x0a\x10\xd0\x05\x18\x00'))
    init = packet(stream)
    assert init[1:] == (0x10008, None, b'\x08\x80\x0a\x10\xd0\x05\x18\x1e'), init
    output.mkdir(parents=True, exist_ok=True)
    videos, audio, stamps = [], [], {}
    audio_init = False
    stream.sendall(lab.command(0x18009))
    limit = time.monotonic() + 3.2  # crosses the two-second asset loop
    while time.monotonic() < limit:
        channel, kind, stamp, payload = packet(stream)
        if kind == 0x30001:
            assert payload == b'\x08\xc4\xd8\x02\x10\x02\x18\x10'
            audio_init = True
        elif kind in (0x20001, 0x30006):
            assert stamp >= stamps.get(channel, 0), (stamp, stamps)
            stamps[channel] = stamp
            if kind == 0x20001:
                assert payload.startswith(b'\0\0\0\1')
                videos.append(payload)
            else:
                assert audio_init and len(payload) == 3528
                audio.append(payload)
        else:
            assert kind == 0x10026, hex(kind)
    assert len(videos) >= 25 and len(audio) >= 70, (len(videos), len(audio))
    assert videos[0][4] & 31 == 7
    (output / 'received.h264').write_bytes(b''.join(videos))
    (output / 'received.pcm').write_bytes(b''.join(audio))
    samples = struct.unpack('<' + 'h' * (len(b''.join(audio)) // 2), b''.join(audio))
    assert 100 < max(abs(v) for v in samples) < 1500, 'quiet PCM tone amplitude invalid'

    # RESET retains the negotiated mode and requests a fresh keyframe.
    time.sleep(.5)  # Deliberately fill the bounded USB media queue first.
    stream.sendall(lab.command(0x1800b))
    limit = time.monotonic() + 2
    while True:
        assert time.monotonic() < limit, 'RESET did not produce a keyframe'
        item = packet(stream)
        if item[1] == 0x20001 and item[3][4] & 31 == 7:
            break
    stream.sendall(lab.command(0x1800a))
    limit = time.monotonic() + 2
    while True:
        assert time.monotonic() < limit, 'PAUSE did not stop audio'
        if packet(stream)[1] == 0x30002:
            break
    status = packet(stream)
    assert status[1] == 0x10026 and status[3][-1] == 0, status
    lab.no_response(stream)
    stream.settimeout(3)
    heartbeat = packet(stream)
    assert heartbeat[1] == 0x20002 and not heartbeat[3], heartbeat

    # Resume without another INIT; first resumed video must carry SPS/PPS/IDR.
    stream.sendall(lab.command(0x18009))
    while True:
        item = packet(stream)
        if item[1] == 0x20001:
            assert item[3][4] & 31 == 7
            break
    # Music STOP must leave video running; a subsequent status query reports 0.
    stream.sendall(lab.command(0x18028, b'\x08\x03\x10\x00'))
    limit = time.monotonic() + 2
    while True:
        assert time.monotonic() < limit
        if packet(stream)[1] == 0x30002:
            break
    # Drain bounded in-flight data, then verify query doesn't restart audio.
    stream.sendall(lab.command(0x18028, b'\x08\x03\x10\x01'))
    limit = time.monotonic() + .4
    query_answer = False
    while time.monotonic() < limit:
        item = packet(stream)
        assert item[1] != 0x30006, 'music query resumed PCM'
        if item[1] == 0x10026:
            assert item[3][-1] == 0
            query_answer = True
    assert query_answer
    stream.sendall(lab.command(0x1800a) + lab.command(0x18009))
    limit = time.monotonic() + .4
    while time.monotonic() < limit:
        item = packet(stream)
        assert item[1] not in (0x30001, 0x30006), 'video resume overrode HU music stop'
    print(json.dumps({'event': 'media_probe', 'status': 'passed', 'video_frames': len(videos),
                      'pcm_chunks': len(audio), 'output': str(output.resolve()),
                      'checks': ['fps_zero', 'loop', 'backpressure_reset', 'pause_heartbeat', 'resume', 'music_stop_query', 'mute_latch']}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument('--usb', action='store_true')
    source.add_argument('--address')
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    if args.usb:
        usb = load('test-carlife-usb')
        stream = usb.BulkStream(usb.accessory())
    else:
        host, port = args.address.rsplit(':', 1)
        stream = socket.create_connection((host, int(port)), timeout=3)
    try:
        probe(stream, args.output)
    finally:
        stream.close()
