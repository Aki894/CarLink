"""CI-only synthetic OXP -> real forwarding process -> CarLife TCP HU contract."""
import base64
import importlib.util
import json
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

root = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(root / 'receiver'))
import probe


def load(name):
    spec = importlib.util.spec_from_file_location(name, root / 'scripts' / (name + '.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


lab, media = load('test-carlife-lab'), load('test-carlife-media')


def wait_path(path):
    deadline = time.monotonic() + 5
    while not path.exists():
        if time.monotonic() > deadline:
            raise AssertionError('CI socket did not appear')
        time.sleep(.02)


with tempfile.TemporaryDirectory(prefix='carlink-live-ci-') as directory:
    directory = Path(directory)
    live, oxp = directory / 'live', directory / 'oxp'
    app, audio = map(lambda p: Path(p).resolve(), sys.argv[1:3])
    wrapper = directory / 'audio'
    wrapper.write_text('#!/bin/sh\nexec qemu-arm -L /usr/arm-linux-gnueabihf "' + str(audio) + '" "$@"\n')
    wrapper.chmod(0o755)
    reserve = socket.socket()
    reserve.bind(('127.0.0.1', 0))
    port = reserve.getsockname()[1]
    reserve.close()
    children = []
    errors = []
    try:
        children.append(subprocess.Popen([str(app), 'carlife', 'tcp', '--listen', f'127.0.0.1:{port}',
            '--duration', '20', '--live-socket', str(live)], stdout=subprocess.DEVNULL))
        deadline = time.monotonic() + 5
        while True:
            try:
                hu = socket.create_connection(('127.0.0.1', port), timeout=2)
                break
            except ConnectionRefusedError:
                assert time.monotonic() < deadline
                time.sleep(.02)
        with hu:
            hu.settimeout(5)
            hu.sendall(lab.command(0x18001, b'\x08\x02\x10\x00'))
            for _ in range(4):
                media.packet(hu)
            config = b'\x08\xa0\x06\x10\xe0\x03\x18\x1e'
            hu.sendall(lab.command(0x18007, config))
            assert media.packet(hu)[3] == config
            hu.sendall(lab.command(0x18009))
            wait_path(live)
            children.append(subprocess.Popen([sys.executable, str(root / 'receiver/forward.py'), '--unix', str(oxp),
                '--live-socket', str(live), '--audio-binary', str(wrapper)], stdout=subprocess.DEVNULL))
            wait_path(oxp)
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as source:
                source.settimeout(5)
                source.connect(str(oxp))
                probe.send_json(source, dict(type='hello', version=0, role='producer'))
                assert probe.receive(source)['role'] == 'consumer'
                assert probe.receive(source)['displays']['0']['frameRate'] == 'FPS_30'
                probe.send_json(source, dict(type='session', session=dict(phase='ACTIVE')))
                avcc = bytes.fromhex('0164001fffe10002676401000268ee')
                probe.send_json(source, {'type': 'video-format', 'screen': 0, 'format': {
                    'codec': 'H264', 'config': base64.b64encode(avcc).decode(), 'width': 800, 'height': 480, 'fpsHint': 30}})
                probe.send_json(source, {'type': 'audio-format', 'stream': 0, 'format': {
                    'role': 'MEDIA', 'codec': 'PCM_S16LE', 'sampleRate': 48000, 'channels': 2}})
                pcm = struct.pack('<hh', 1200, -1200) * 960
                access_unit = b'\x65\x80'
                expected_video = probe.avc_parameters(avcc) + b'\0\0\0\1' + access_unit

                def send_media():
                    try:
                        for seq in range(50):
                            pts = 1000000 + seq * 20000
                            for kind, payload, flags in [(1, struct.pack('>I', len(access_unit)) + access_unit, 1), (2, pcm, 0)]:
                                frame = probe.HEADER.pack(kind, 0, flags, 0, seq, pts, pts) + payload
                                source.sendall(struct.pack('<I', len(frame)) + frame)
                            time.sleep(.02)
                        probe.send_json(source, {'type': 'audio-format', 'stream': 0, 'format': None})
                    except Exception as e:
                        errors.append(e)

                sender = threading.Thread(target=send_media)
                sender.start()
                seen = set()
                stamps = {}
                while not {0x20001, 0x30001, 0x30006, 0x30002} <= seen:
                    channel, kind, stamp, payload = media.packet(hu)
                    seen.add(kind)
                    if kind == 0x20001:
                        assert payload == expected_video
                    elif kind == 0x30006:
                        assert len(payload) == 3528 and any(payload)
                    if stamp is not None:
                        assert stamp >= stamps.get(channel, 0)
                        stamps[channel] = stamp
                sender.join(3)
                assert not sender.is_alive() and not errors, errors
                hu.sendall(lab.command(0x68002, b'\x08\x90\x03\x10\xf0\x01', channel=6))
                while True:
                    event = probe.receive(source)
                    if isinstance(event, tuple) and event[0] == 16:
                        ident, down, x, y = struct.unpack('<BBHH', event[-1][1:])
                        assert ident == 0 and down == 1 and (x, y) == (32768, 32768)
                        break
                # Lexus legacy CMD-range ids are carried on the TOUCH channel.
                hu.sendall(lab.command(0x1005a, b'\x08\x01', channel=6))
                hu.sendall(lab.command(0x1005b, b'\x08\x02\x10'+b'\xd0'+b'\xff'*8+b'\x01\x18\x00', channel=6))
                keys = []
                while len(keys) < 2:
                    event = probe.receive(source)
                    if isinstance(event, tuple) and event[0] == 17:
                        keys.append(struct.unpack('<HB', event[-1]))
                assert keys == [(3, 1), (3, 0)], keys
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
        for child in children:
            try:
                child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=3)
print('Synthetic OXP -> ARM audio resampler -> CarLife media + reverse touch passed')
