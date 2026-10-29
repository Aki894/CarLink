"""CI host Rust OXP consumer against the existing Python wire reference.

Usage: python3 scripts/test-carplay-output.py HOST_CARLINK_BINARY
No codec, Bluetooth, USB, phone or real Opus library is required. A host C
compiler builds a temporary failure-injection library during Linux CI only.
"""
import base64
import contextlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'receiver'))
import probe

APP = Path(sys.argv[1]).resolve()


def live_frame(kind, payload=b'', flags=0):
    body = struct.pack('<BBHQ', kind, flags, 0, 0) + payload
    return struct.pack('<I', len(body)) + body


def live_receive(sock):
    size = struct.unpack('<I', probe.exact(sock, 4))[0]
    assert 12 <= size <= 1024 * 1024, 'invalid mock live length'
    body = probe.exact(sock, size)
    kind, flags, reserved, pts = struct.unpack_from('<BBHQ', body)
    assert reserved == 0
    return kind, flags, pts, bytes(body[12:])


def live_until(sock, predicate, timeout=4):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        sock.settimeout(max(.01, deadline - time.monotonic()))
        value = live_receive(sock)
        if predicate(value):
            return value
    raise AssertionError('live frame deadline')


def oxp_until(sock, predicate, timeout=4):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        sock.settimeout(max(.01, deadline - time.monotonic()))
        value = probe.receive(sock)
        if predicate(value):
            return value
    raise AssertionError('OXP frame deadline')


def send_fragmented(sock, value):
    body = json.dumps(value, separators=(',', ':')).encode()
    wire = struct.pack('<I', len(body)) + body
    # Exercise boundaries within the length prefix, JSON and adjacent frames.
    for byte in wire:
        sock.sendall(bytes([byte]))


@contextlib.contextmanager
def harness(opus_library=None):
    with tempfile.TemporaryDirectory(prefix='carlink-output-ci-') as folder:
        folder = Path(folder)
        live_path, oxp_path = folder / 'live', folder / 'oxp'
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(str(live_path)); listener.listen(2); listener.settimeout(5)
        log = (folder / 'child.log').open('wb')
        command = [str(APP), 'carplay-output', '--unix', str(oxp_path),
            '--live-socket', str(live_path), '--audio-binary', '/nonexistent/codec-forbidden',
            '--width', '800', '--height', '480']
        if opus_library is not None:
            command.extend(['--opus-library', str(opus_library)])
        child = subprocess.Popen(command, stdout=log, stderr=log,
            env=dict(os.environ, CARLINK_LOW_LATENCY='1'))
        source = sink = None
        try:
            deadline = time.monotonic() + 6
            while not oxp_path.exists():
                assert child.poll() is None, 'Rust output exited before bind'
                assert time.monotonic() < deadline, 'Rust output did not bind'
                time.sleep(.01)
            source = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            source.settimeout(5); source.connect(str(oxp_path))
            send_fragmented(source, dict(type='hello', version=0, role='producer'))
            hello, caps = probe.receive(source), probe.receive(source)
            assert hello['type'] == 'hello' and hello['role'] == 'consumer' and hello['version'] == 0
            assert hello['client'].endswith(';carlink-host-ui=1')
            assert caps['type'] == 'capabilities' and caps['microphone'] is True and caps['prefersH264'] is True
            assert caps['displays']['0']['frameRate'] == 'FPS_30'
            assert caps['displays']['0']['widthPx'] == 800 and caps['displays']['0']['heightPx'] == 480
            assert caps['keys'] == list(range(11)) and caps['touch'] == 10
            sink, _ = listener.accept(); sink.settimeout(5)
            yield child, source, sink, listener, oxp_path
        except BaseException:
            log.flush()
            print((folder / 'child.log').read_text(errors='replace')[-4000:], file=sys.stderr)
            raise
        finally:
            if source is not None: source.close()
            if sink is not None: sink.close()
            listener.close()
            if child.poll() is None: child.terminate()
            try: child.wait(timeout=5)
            except subprocess.TimeoutExpired: child.kill(); child.wait(timeout=3)
            log.close()


def active_video(source):
    probe.send_json(source, dict(type='session', session=dict(phase='ACTIVE')))
    raw = bytes.fromhex('0164001fffe10002676401000268ee')
    probe.send_json(source, {'type': 'video-format', 'screen': 0, 'format': {
        'codec': 'H264', 'config': base64.b64encode(raw).decode(), 'width': 800, 'height': 480, 'fpsHint': 30}})
    return probe.avc_parameters(raw)


with harness() as (child, source, sink, listener, path):
    parameters = active_video(source)
    assert live_until(sink, lambda f: f[0] == 1)[3] == struct.pack('<III', 800, 480, 30)
    def video(sequence, nal, age=0):
        payload = struct.pack('>I', len(nal)) + nal
        now = time.monotonic_ns() // 1000
        header = probe.HEADER.pack(1, 0, 1, 0, sequence, 1000 + sequence, now - age)
        source.sendall(struct.pack('<I', len(header) + len(payload)) + header + payload)
    video(1, b'\x65\xff', 2_000_000)
    video(2, b'\x65\x80')
    frame = live_until(sink, lambda f: f[0] == 2)
    assert frame[3] == parameters + b'\0\0\0\1\x65\x80', 'stale or incorrectly converted video'

    # Native OEM menu requests keep the audio/video session alive. The mux
    # return notification is intentionally not forwarded to stock OXP clients.
    probe.send_json(source, {'type': 'host-ui-request'})
    assert live_until(sink, lambda f: f[0] == 8) == (8, 0, 0, b'')
    sink.sendall(live_frame(129, json.dumps({'type': 'return-projection'}).encode()))
    video(3, b'\x65\x81')
    assert live_until(sink, lambda f: f[0] == 2)[3].endswith(b'\x65\x81')
    assert child.poll() is None

    # Feedback crosses the actual process in the opposite direction.
    for value in [{'type': 'key-press', 'code': 3},
                  {'type': 'touch', 'contacts': [{'id': 0, 'x': .5, 'y': 1, 'down': True}]}]:
        sink.sendall(live_frame(129, json.dumps(value).encode()))
    down = oxp_until(source, lambda v: isinstance(v, tuple) and v[0] == 17)
    up = oxp_until(source, lambda v: isinstance(v, tuple) and v[0] == 17)
    touch = oxp_until(source, lambda v: isinstance(v, tuple) and v[0] == 16)
    assert down[-1] == b'\x03\0\1' and up[-1] == b'\x03\0\0'
    assert touch[-1] == b'\x01\x00\x01\x00\x80\xff\xff'

    # TELEPHONY activation/stop needs no compressed input or codec process.
    probe.send_json(source, {'type': 'audio-format', 'stream': 4, 'format': {
        'role': 'TELEPHONY', 'codec': 'PCM_S16LE', 'sampleRate': 16000, 'channels': 1}})
    live_until(sink, lambda f: f[0] == 5 and f[1] == 3 and f[3] == b'\1')
    probe.send_json(source, {'type': 'audio-format', 'stream': 4, 'format': None})
    live_until(sink, lambda f: f[0] == 5 and f[1] == 3 and f[3] == b'\0')

    probe.send_json(source, {'type': 'mic', 'stream': 7, 'request': {
        'codec': 'PCM_S16LE', 'sampleRate': 16000, 'channels': 1, 'frameMs': 20}})
    request = json.loads(live_until(sink, lambda f: f[0] == 7 and json.loads(f[3])['active'])[3])
    generation = request['generation']
    old, pcm = struct.pack('<h', 111) * 320, struct.pack('<h', 222) * 320
    sink.sendall(live_frame(130, struct.pack('<Q', generation - 1) + old)
                 + live_frame(130, struct.pack('<Q', generation) + pcm))
    microphone = oxp_until(source, lambda v: isinstance(v, tuple) and v[0] == 3)
    assert microphone[1] == 7 and microphone[-1] == pcm, 'stale microphone generation leaked'
    probe.send_json(source, {'type': 'mic', 'stream': 7, 'request': None})
    stopped = json.loads(live_until(sink, lambda f: f[0] == 7 and not json.loads(f[3])['active'])[3])
    assert stopped['generation'] == generation
    source.shutdown(socket.SHUT_WR)
    child.wait(timeout=5)
    assert not path.exists(), 'OXP socket retained after EOF'

# A real FFI encode failure must stop microphone capture without killing video.
# The fixture exports the actual Opus ABI; no system codec or device is used.
with tempfile.TemporaryDirectory(prefix='carlink-opus-failure-ci-') as folder:
    folder = Path(folder)
    fixture, library = folder / 'opus-failure.c', folder / 'opus-failure.so'
    fixture.write_text(r'''
#include <stdint.h>
void *opus_encoder_create(int32_t rate, int channels, int application, int *error) {
    (void)rate; (void)channels; (void)application;
    *error = 0;
    return (void *)(uintptr_t)1;
}
int opus_encoder_ctl(void *handle, int request, ...) {
    (void)handle; (void)request;
    return 0;
}
int opus_encode(void *handle, const int16_t *pcm, int frame_size,
                unsigned char *output, int32_t capacity) {
    (void)handle; (void)pcm; (void)frame_size; (void)output; (void)capacity;
    return -1;
}
void opus_encoder_destroy(void *handle) { (void)handle; }
''', encoding='ascii')
    subprocess.run(['gcc', '-shared', '-fPIC', str(fixture), '-o', str(library)], check=True)
    with harness(library) as (child, source, sink, listener, path):
        parameters = active_video(source)
        live_until(sink, lambda f: f[0] == 1)

        def send_idr(sequence):
            nal = bytes([0x65, sequence])
            payload = struct.pack('>I', len(nal)) + nal
            now = time.monotonic_ns() // 1000
            header = probe.HEADER.pack(1, 0, 1, 0, sequence, 1000 + sequence, now)
            source.sendall(struct.pack('<I', len(header) + len(payload)) + header + payload)
            received = live_until(sink, lambda f: f[0] == 2)
            assert received[3] == parameters + b'\0\0\0\1' + nal

        send_idr(1)
        probe.send_json(source, {'type': 'mic', 'stream': 3, 'request': {
            'codec': 'OPUS', 'sampleRate': 24000, 'channels': 1, 'frameMs': 20}})
        request = json.loads(live_until(sink, lambda f: f[0] == 7 and json.loads(f[3])['active'])[3])
        generation = request['generation']
        # 512 mono samples at 16 kHz produce over 960 resampled samples,
        # forcing one actual encode() invocation and its injected error.
        capture = struct.pack('<h', 731) * 512
        sink.sendall(live_frame(130, struct.pack('<Q', generation) + capture))
        stopped = json.loads(live_until(sink, lambda f: f[0] == 7 and not json.loads(f[3])['active'])[3])
        assert stopped['generation'] == generation
        assert child.poll() is None, 'native microphone error killed output process'
        send_idr(2)
        assert child.poll() is None, 'video did not survive native microphone error'

# Invalid configuration and truncated/oversized wire input must terminate only
# this output process and remove its owned socket, without a codec/device.
for case in ('invalid-format', 'truncated', 'oversized'):
    with harness() as (child, source, sink, listener, path):
        if case == 'invalid-format':
            probe.send_json(source, dict(type='session', session=dict(phase='ACTIVE')))
            probe.send_json(source, {'type': 'video-format', 'screen': 0, 'format': {
                'codec': 'H265', 'config': '', 'width': 800, 'height': 480}})
        elif case == 'truncated':
            source.sendall(struct.pack('<I', 100) + b'{"type":'); source.shutdown(socket.SHUT_WR)
        else:
            source.sendall(struct.pack('<I', probe.MAX_FRAME + 1))
        child.wait(timeout=5)
        assert child.returncode != 0, case
        assert not path.exists(), case + ': OXP socket retained'

print('Rust CarPlay output: real OXP hello/capabilities, binary video, stale-frame fencing, reverse input, PCM microphone generations, native microphone error isolation, call stop and malformed/EOF cleanup passed')
