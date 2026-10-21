#!/usr/bin/env python3
"""Bounded OXP v0 consumer for CarLink's headless receiver validation."""
import argparse
import base64
import json
import math
import os
from pathlib import Path
import selectors
import socket
import struct
import time

MAX_FRAME = 16 * 1024 * 1024
MAX_JSON = 256 * 1024
HEADER = struct.Struct('<BBBBIqq')


def exact(stream, size):
    data = bytearray()
    while len(data) < size:
        chunk = stream.recv(size - len(data))
        if not chunk:
            raise EOFError('wire closed')
        data.extend(chunk)
    return data


def receive(stream):
    size = struct.unpack('<I', exact(stream, 4))[0]
    if not 1 <= size <= MAX_FRAME:
        raise ValueError('invalid wire length')
    data = exact(stream, size)
    if data[0] == ord('{'):
        if size > MAX_JSON:
            raise ValueError('JSON frame too large')
        value = json.loads(data)
        if not isinstance(value, dict):
            raise ValueError('expected JSON object')
        return value
    if size < HEADER.size:
        raise ValueError('truncated binary header')
    kind, screen, flags, reserved, sequence, pts, sender = HEADER.unpack_from(data)
    if kind not in (1, 2, 3, 16, 17) or reserved or flags > 3:
        raise ValueError('invalid binary header')
    return (kind, screen, flags, sequence, pts, sender, data[HEADER.size:])


def send_json(stream, value):
    data = json.dumps(value, separators=(',', ':'), allow_nan=False).encode()
    stream.sendall(struct.pack('<I', len(data)) + data)


def annexb(data):
    """OXP always uses four-byte big-endian NAL lengths."""
    result = bytearray()
    pos = 0
    while pos < len(data):
        if len(data) - pos < 4:
            raise ValueError('truncated NAL length')
        size = struct.unpack_from('>I', data, pos)[0]
        pos += 4
        if size == 0 or size > len(data) - pos:
            raise ValueError('invalid NAL length')
        result.extend(b'\0\0\0\1')
        result.extend(data[pos:pos + size])
        pos += size
    if not result:
        raise ValueError('empty video frame')
    return result


def avc_parameters(data):
    if len(data) < 7 or data[0] != 1 or data[4] & 3 != 3:
        raise ValueError('expected AVC configuration with four-byte NAL lengths')
    pos, result = 6, bytearray()
    counts = [data[5] & 31]
    if not counts[0]:
        raise ValueError('missing SPS')
    for group in range(2):
        if group:
            if pos >= len(data):
                raise ValueError('missing PPS count')
            counts = [data[pos]]
            pos += 1
        for _ in range(counts[0]):
            if pos + 2 > len(data):
                raise ValueError('truncated AVC parameter length')
            size = int.from_bytes(data[pos:pos + 2], 'big')
            pos += 2
            if not size or size > len(data) - pos:
                raise ValueError('invalid AVC parameter length')
            result.extend(b'\0\0\0\1' + data[pos:pos + size])
            pos += size
    if counts[0] == 0 or not result:
        raise ValueError('missing AVC parameters')
    return result


def control(stream, value):
    if value == {'type': 'keyframe', 'screen': 0}:
        send_json(stream, value)
        return
    if set(value) != {'type', 'contacts'} or value['type'] != 'touch':
        raise ValueError('expected keyframe or touch control')
    contacts = value['contacts']
    if not isinstance(contacts, list) or len(contacts) > 10:
        raise ValueError('invalid contacts')
    payload, seen = bytearray([len(contacts)]), set()
    for c in contacts:
        if set(c) != {'id', 'x', 'y', 'down'}:
            raise ValueError('invalid contact fields')
        ident, x, y, down = c['id'], c['x'], c['y'], c['down']
        if (type(ident) is not int or not 0 <= ident <= 9 or ident in seen
                or type(down) is not bool
                or any(type(n) not in (int, float) or not math.isfinite(n) or not 0 <= n <= 1 for n in (x, y))):
            raise ValueError('invalid contact values')
        seen.add(ident)
        payload.extend(struct.pack('<BBHH', ident, down, int(x * 65535 + .5), int(y * 65535 + .5)))
    now = time.monotonic_ns() // 1000
    header = HEADER.pack(16, 0, 0, 0, 0, now, now)
    stream.sendall(struct.pack('<I', len(header) + len(payload)) + header + payload)


def listener(path):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    # Never unlink a path owned by another service; callers clean stale sockets.
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        s.bind(str(path))
        os.chmod(path, 0o600)
        s.listen(1)
        return s
    except BaseException:
        s.close()
        raise


def run(args):
    status_path = Path(args.status)
    status_path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    state = dict(schema=1, connected=False, video_frames=0, video_bytes=0,
                 keyframes=0, audio_packets=0, audio_bytes=0, written_bytes=0,
                 recording_limit_reached=False, controls_sent=0, video_format=None,
                 audio_formats={}, last_video=None)

    def status():
        state['updated_monotonic_ns'] = time.monotonic_ns()
        temporary = status_path.with_suffix('.tmp')
        temporary.write_text(json.dumps(state, separators=(',', ':')) + '\n')
        os.chmod(temporary, 0o600)
        temporary.replace(status_path)

    media, ctl, stream, recording = None, None, None, None
    control_path = args.control_socket or args.unix + '.control'
    segment, parameters, active, awaiting_keyframe = 0, b'', False, True
    limit = args.max_record_mib * 1024 * 1024
    record_dir = Path(args.record_directory) if args.record_directory else None
    if record_dir:
        record_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
        state['written_bytes'] = sum(p.stat().st_size for p in record_dir.glob('video-*.h264') if p.is_file())
        state['recording_limit_reached'] = state['written_bytes'] >= limit
    try:
        media = listener(args.unix)
        ctl = listener(control_path)
        status()
        stream, _ = media.accept()
        stream.settimeout(5)
        hello = receive(stream)
        if not isinstance(hello, dict) or hello.get('type') != 'hello' or hello.get('role') != 'producer' or hello.get('version') != 0:
            raise ValueError('expected OXP v0 producer hello')
        send_json(stream, dict(type='hello', version=0, role='consumer', name='CarLink probe', client='carlink-probe'))
        send_json(stream, dict(type='capabilities', latency='INTERACTIVE',
                              decoders=[dict(codecString='avc1.42E01E', hardware=False)],
                              displays={'0': dict(widthPx=args.width, heightPx=args.height,
                                  densityDpi=160, refreshHz=30, safeArea=dict(left=0, top=0, right=0, bottom=0))},
                              touch=10, keys=[], audio=[], microphone=False, prefersH264=True))
        state['connected'] = True
        with selectors.DefaultSelector() as selector:
            selector.register(stream, selectors.EVENT_READ, 'media')
            selector.register(ctl, selectors.EVENT_READ, 'control')
            last_status = 0.0
            while True:
                for key, _ in selector.select(1):
                    if key.data == 'control':
                        with ctl.accept()[0] as client:
                            client.settimeout(1)
                            try:
                                raw = bytearray()
                                while b'\n' not in raw and len(raw) < 4096:
                                    chunk = client.recv(min(4096 - len(raw), 1024))
                                    if not chunk:
                                        break
                                    raw.extend(chunk)
                                if b'\n' not in raw:
                                    raise ValueError('control requires one bounded JSON line')
                                control(stream, json.loads(raw.split(b'\n', 1)[0]))
                                state['controls_sent'] += 1
                                client.sendall(b'{"ok":true}\n')
                            except (ValueError, TypeError, KeyError, socket.timeout):
                                client.sendall(b'{"ok":false}\n')
                        continue
                    message = receive(stream)
                    if isinstance(message, dict):
                        kind = message.get('type')
                        if kind == 'session':
                            active = message.get('session', {}).get('phase') == 'ACTIVE'
                            state['session_phase'] = message.get('session', {}).get('phase')
                            if not active:
                                state['video_format'], parameters, awaiting_keyframe = None, b'', True
                                if recording:
                                    recording.close()
                                    recording = None
                        elif kind == 'video-format' and message.get('screen') == 0:
                            f = message.get('format')
                            parameters = b''
                            awaiting_keyframe = True
                            if recording:
                                recording.close()
                                recording = None
                            if f is not None:
                                if f.get('codec') != 'H264':
                                    raise ValueError('probe requires H264; select H264 in receiver settings')
                                parameters = avc_parameters(base64.b64decode(f['config'], validate=True))
                                state['video_format'] = {k: f.get(k) for k in ('codec', 'width', 'height', 'fpsHint')}
                                send_json(stream, {'type': 'keyframe', 'screen': 0})
                            else:
                                state['video_format'] = None
                        elif kind == 'audio-format':
                            f = message.get('format')
                            state['audio_formats'][str(message.get('stream'))] = (
                                {k: f.get(k) for k in ('role', 'codec', 'sampleRate', 'channels')} if f else None)
                    else:
                        kind, screen, flags, seq, pts, sender, payload = message
                        if kind == 1:
                            if not active or screen != 0 or not parameters:
                                continue
                            frame = annexb(payload)
                            state['video_frames'] += 1
                            state['video_bytes'] += len(payload)
                            state['keyframes'] += bool(flags & 1)
                            state['last_video'] = dict(sequence=seq, pts_us=pts, sender_clock_us=sender, keyframe=bool(flags & 1))
                            if record_dir and not state['recording_limit_reached']:
                                if awaiting_keyframe and not flags & 1:
                                    continue
                                extra = parameters if recording is None else b''
                                if state['written_bytes'] + len(extra) + len(frame) > limit:
                                    state['recording_limit_reached'] = True
                                    if recording:
                                        recording.close()
                                        recording = None
                                else:
                                    if recording is None:
                                        segment += 1
                                        path = record_dir / f'video-{time.time_ns()}-{segment}.h264'
                                        recording = path.open('xb')
                                        os.chmod(path, 0o600)
                                        recording.write(extra)
                                    recording.write(frame)
                                    awaiting_keyframe = False
                                    state['written_bytes'] += len(extra) + len(frame)
                        elif kind == 2:
                            state['audio_packets'] += 1
                            state['audio_bytes'] += len(payload)
                if time.monotonic() - last_status >= 1:
                    if recording:
                        recording.flush()
                    status()
                    last_status = time.monotonic()
    except (EOFError, OSError, ValueError, KeyError, TypeError) as error:
        # Never emit producer hello, credentials, media bytes or arbitrary exception text.
        state['error'] = type(error).__name__
        raise
    finally:
        state['connected'] = False
        status()
        if recording:
            recording.close()
        if stream:
            stream.close()
        for server, path in ((ctl, control_path), (media, args.unix)):
            if server is not None:
                server.close()
                Path(path).unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--unix', default='/run/carlink/oxp-probe.sock')
    parser.add_argument('--status', default='/run/carlink/carplay-probe.json')
    parser.add_argument('--control-socket')
    parser.add_argument('--record-directory', default=os.environ.get('CARLINK_PROBE_RECORD_DIR'))
    parser.add_argument('--max-record-mib', type=int, default=128)
    parser.add_argument('--width', type=int, default=800)
    parser.add_argument('--height', type=int, default=480)
    parser.add_argument('--send-control', metavar='PATH', help='send --command to existing probe control socket')
    parser.add_argument('--command', default='{"type":"keyframe","screen":0}')
    args, _ = parser.parse_known_args()  # oxp-relay supervisor output flags are intentionally ignored
    if not 1 <= args.max_record_mib <= 1024 or not 1 <= args.width <= 4096 or not 1 <= args.height <= 4096:
        parser.error('invalid record limit or display dimensions')
    if args.send_control:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(3)
            s.connect(args.send_control)
            s.sendall(args.command.encode() + b'\n')
            response = s.recv(128)
            print(response.decode().strip())
            return 0 if response == b'{"ok":true}\n' else 1
    os.umask(0o077)
    try:
        run(args)
    except (EOFError, OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({'type': 'carlink-probe-exit', 'error': type(error).__name__}), flush=True)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
