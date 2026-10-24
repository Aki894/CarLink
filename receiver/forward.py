#!/usr/bin/env python3
"""Bounded OXP consumer: H.264 passthrough, AAC/PCM conversion, CarLife controls."""
import argparse
import base64
from collections import deque
import json
import os
from pathlib import Path
import queue
import select
import signal
import socket
import struct
import subprocess
import threading
import time

LIVE = struct.Struct('<BBHQ')
MAX_VIDEO = 512 * 1024
MAX_QUEUE = 1024 * 1024
VIDEO_BUDGET = .12
AUDIO_BUDGET = .12
AUDIO_DECODE_BUDGET = .5  # Compressed AirPlay audio arrives in bursts before PCM pacing.
PCM_CHUNK = 3528  # 20 ms, 44100 Hz, stereo S16LE
ROLES = {'MEDIA': 0, 'ALERT': 1}


def packet(kind, payload=b'', flags=0, pts=0):
    if kind not in (1, 2, 3, 4, 5, 6) or not 0 <= flags <= 255 or not 0 <= pts < 1 << 64:
        raise ValueError('invalid live header')
    if len(payload) > (MAX_VIDEO if kind == 2 else 65536):
        raise ValueError('live payload too large')
    body = LIVE.pack(kind, flags, 0, pts) + payload
    return struct.pack('<I', len(body)) + body


def adts(payload, rate, channels):
    rates = (96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000)
    if rate not in rates or channels not in (1, 2) or not 0 < len(payload) <= 8184:
        raise ValueError('unsupported AAC access unit')
    size, index = len(payload) + 7, rates.index(rate)
    return bytes((255, 241, (1 << 6) | (index << 2) | (channels >> 2),
                  ((channels & 3) << 6) | (size >> 11), (size >> 3) & 255,
                  ((size & 7) << 5) | 31, 252)) + payload


def ogg_page(payload, sequence, granule, bos=False):
    """One complete packet per page; Ogg CRC uses the forward polynomial."""
    if not 0 < len(payload) < 65025:
        raise ValueError('invalid Ogg packet length')
    lacing = bytes([255] * (len(payload) // 255) + [len(payload) % 255])
    page = bytearray(b'OggS' + bytes([0, 2 if bos else 0]) +
        struct.pack('<QIIIB', granule, 1, sequence, 0, len(lacing)) + lacing + payload)
    crc = 0
    for value in page:
        crc ^= value << 24
        for _ in range(8):
            crc = ((crc << 1) ^ (0x04c11db7 if crc & 0x80000000 else 0)) & 0xffffffff
    struct.pack_into('<I', page, 22, crc)
    return page


def opus_samples(payload):
    if not payload:
        raise ValueError('empty Opus packet')
    toc = payload[0]
    if toc & 128:
        duration = 120 << ((toc >> 3) & 3)
    elif toc & 96 == 96:
        duration = 960 if toc & 8 else 480
    else:
        duration = (480, 960, 1920, 2880)[(toc >> 3) & 3]
    count = (1, 2, 2, payload[1] & 63 if len(payload) > 1 else 0)[toc & 3]
    if not 0 < duration * count <= 5760:
        raise ValueError('invalid Opus duration')
    return duration * count


class LiveLink:
    """Single socket owner; disconnects never block the OXP receive thread."""
    def __init__(self, path, feedback):
        self.path, self.feedback = path, feedback
        self.lock = threading.Lock()
        self.pending = deque()
        self.size = 0
        self.active = False
        self.config = None
        self.connected = False
        self.wait_key = True
        self.closed = threading.Event()
        self.sent = self.dropped = self.reconnects = 0
        self.last_recovery = 0.
        self.queue_ms_max = self.send_ms_max = self.source_ms_max = 0.
        self.source_dropped = 0
        self.video_frames = self.video_bytes = 0
        self.report_at = time.monotonic()
        self.thread = threading.Thread(target=self._run, daemon=True)

    def start(self):
        self.thread.start()

    def _clear(self):
        self.pending.clear()
        self.size = 0
        self.wait_key = True

    def _put(self, kind, payload=b'', flags=0, pts=0):
        wire = packet(kind, payload, flags, pts)
        self.pending.append((kind, wire, time.monotonic()))
        self.size += len(wire)

    def session(self, active):
        with self.lock:
            self.active = active
            self._clear()
            self._put(4, flags=int(active))
            if self.config:
                self._put(1, self.config)

    def configure(self, width, height, fps):
        if not (16 <= width <= 1920 and 16 <= height <= 1080 and width % 2 == height % 2 == 0 and 1 <= fps <= 60):
            raise ValueError('unsupported dimensions')
        with self.lock:
            self.config = struct.pack('<III', width, height, fps)
            self._clear()
            self._put(4, flags=int(self.active))
            self._put(1, self.config)

    def audio_state(self, role, active):
        with self.lock:
            # Remove queued audio for the ended/reconfigured role.
            kept = [(k, w, t) for k, w, t in self.pending if k != 3 or w[5] != role]
            self.pending = deque(kept)
            self.size = sum(len(w) for _, w, _ in kept)
            self._put(5, bytes([int(active)]), flags=role)

    def recover_video(self):
        with self.lock:
            kept = [(k, w, t) for k, w, t in self.pending if k != 2 and k != 6]
            self.pending = deque(kept)
            self.size = sum(len(w) for _, w, _ in kept)
            self.wait_key = True
            self._put(6)  # Video-only fence; audio player remains alive.
            request = time.monotonic() - self.last_recovery >= .25
            if request:
                self.last_recovery = time.monotonic()
        if request:
            self.feedback({'type': 'keyframe', 'screen': 0})

    def report(self):
        with self.lock:
            elapsed = max(.001, time.monotonic() - self.report_at)
            value = dict(event='forward_latency', sent=self.sent, dropped=self.dropped,
                reconnects=self.reconnects, pending=len(self.pending), pending_bytes=self.size,
                queue_ms_max=round(self.queue_ms_max, 2), send_ms_max=round(self.send_ms_max, 2),
                source_video_ms_max=round(self.source_ms_max, 2), source_video_dropped=self.source_dropped,
                source_video_fps=round(self.video_frames / elapsed, 2),
                source_video_mbps=round(self.video_bytes * 8 / elapsed / 1000000, 3))
            self.queue_ms_max = self.send_ms_max = self.source_ms_max = 0.
            self.video_frames = self.video_bytes = 0
            self.report_at = time.monotonic()
        print(json.dumps(value), flush=True)

    def media(self, kind, payload, pts, flags=0):
        request = False
        with self.lock:
            if not self.connected or not self.active or not self.config:
                self.dropped += 1
                return False
            if kind == 2 and self.wait_key and not flags & 1:
                self.dropped += 1
                return False
            limit = 4 if kind == 2 else 6
            if self.size + len(payload) + 16 > MAX_QUEUE or sum(k == kind for k, _, _ in self.pending) >= limit:
                self.dropped += 1
                if kind == 2:
                    request = True
            else:
                self._put(kind, payload, flags, max(0, pts))
                if kind == 2:
                    self.wait_key = False
                return True
        if request:
            self.recover_video()
        return False

    def _run(self):
        while not self.closed.is_set():
            try:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
                    s.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 32768)
                    s.settimeout(.2)
                    s.connect(self.path)
                    with self.lock:
                        self.connected = True
                        self.reconnects += 1
                        self._clear()
                        self._put(4, flags=int(self.active))
                        if self.config:
                            self._put(1, self.config)
                    self.feedback({'type': 'keyframe', 'screen': 0})
                    incoming = bytearray()
                    while not self.closed.is_set():
                        with self.lock:
                            item = self.pending.popleft() if self.pending else None
                            if item:
                                self.size -= len(item[1])
                        if item:
                            kind, wire, queued = item
                            age = time.monotonic() - queued
                            self.queue_ms_max = max(self.queue_ms_max, age * 1000)
                            if kind in (2, 3) and age > (VIDEO_BUDGET if kind == 2 else AUDIO_BUDGET):
                                self.dropped += 1
                                if kind == 2:
                                    self.recover_video()
                            else:
                                started = time.monotonic()
                                s.sendall(wire)
                                self.send_ms_max = max(self.send_ms_max, (time.monotonic() - started) * 1000)
                                self.sent += 1
                        readable, _, _ = select.select([s], [], [], 0 if item else .02)
                        if readable:
                            data = s.recv(4096)
                            if not data:
                                raise EOFError()
                            incoming.extend(data)
                            while len(incoming) >= 4:
                                size = struct.unpack_from('<I', incoming)[0]
                                if not LIVE.size <= size <= 4096:
                                    raise ValueError('invalid live control length')
                                if len(incoming) < size + 4:
                                    break
                                kind, flags, reserved, _ = LIVE.unpack_from(incoming, 4)
                                if kind != 129 or flags or reserved:
                                    raise ValueError('invalid live control header')
                                value = json.loads(incoming[16:4 + size])
                                del incoming[:4 + size]
                                self.feedback(value)
            except (OSError, EOFError, ValueError, TypeError):
                pass
            finally:
                with self.lock:
                    self.connected = False
                    self._clear()
                self.feedback({'type': 'touch', 'contacts': []})
            self.closed.wait(.25)

    def close(self):
        self.closed.set()
        self.thread.join(1)


class AudioDecoder:
    """Persistent audio-only process; at most 16 compressed packets are queued."""
    def __init__(self, executable, fmt, emit):
        self.rate, self.channels = fmt['sampleRate'], fmt['channels']
        self.codec = fmt['codec']
        if self.rate not in (8000, 16000, 24000, 32000, 44100, 48000) or self.channels not in (1, 2):
            raise ValueError('unsupported source audio format')
        if self.codec in ('AAC_LC', 'AAC_ADTS'):
            inputs = ['-f', 'aac']
        elif self.codec == 'PCM_S16LE':
            inputs = ['-f', 's16le', '-ar', str(self.rate), '-ac', str(self.channels)]
        elif self.codec == 'OPUS':
            inputs = ['-f', 'ogg']
        else:
            raise ValueError('unsupported source audio codec')
        command = [executable, '-hide_banner', '-loglevel', 'error', '-nostdin', '-threads', '1',
                   '-probesize', '32', '-analyzeduration', '0', *inputs, '-i', 'pipe:0',
                   '-vn', '-threads', '1', '-ac', '2', '-ar', '44100', '-c:a', 'pcm_s16le',
                   '-f', 's16le', '-flush_packets', '1', 'pipe:1']
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.DEVNULL, bufsize=0)
        self.emit, self.queue = emit, queue.Queue(maxsize=16)
        self.closed = threading.Event()
        self.origin = None
        self.samples = 0
        self.failed = False
        self.ogg_sequence, self.granule = 0, 0
        self.threads = [threading.Thread(target=self._write, daemon=True), threading.Thread(target=self._read, daemon=True)]
        for t in self.threads:
            t.start()

    def feed(self, payload, pts):
        if self.closed.is_set() or self.failed:
            return False
        if self.origin is None:
            self.origin = max(0, pts)
        data = adts(payload, self.rate, self.channels) if self.codec == 'AAC_LC' else payload
        if self.codec == 'OPUS':
            headers = b''
            if self.ogg_sequence == 0:
                head = b'OpusHead' + struct.pack('<BBHIhB', 1, self.channels, 0, 48000, 0, 0)
                tags = b'OpusTags' + struct.pack('<I', 7) + b'CarLink' + struct.pack('<I', 0)
                headers = ogg_page(head, 0, 0, True) + ogg_page(tags, 1, 0)
                self.ogg_sequence = 2
            self.granule += opus_samples(payload)
            data = headers + ogg_page(payload, self.ogg_sequence, self.granule)
            self.ogg_sequence += 1
        if len(data) > 65536 or (self.codec == 'PCM_S16LE' and len(data) % (2 * self.channels)):
            raise ValueError('invalid source audio packet')
        try:
            self.queue.put_nowait((data, time.monotonic()))
            return True
        except queue.Full:
            self.failed = True
            return False

    def _write(self):
        try:
            while not self.closed.is_set():
                try:
                    data, queued = self.queue.get(timeout=.1)
                except queue.Empty:
                    continue
                if time.monotonic() - queued > AUDIO_DECODE_BUDGET:
                    self.failed = True
                    return
                view = memoryview(data)
                while view and not self.closed.is_set():
                    size = self.process.stdin.write(view)
                    if not size:
                        raise BrokenPipeError()
                    view = view[size:]
        except (OSError, ValueError):
            self.failed = True

    def _read(self):
        pending = bytearray()
        playback_origin = None
        try:
            while not self.closed.is_set():
                data = self.process.stdout.read(PCM_CHUNK)
                if not data:
                    break
                pending.extend(data)
                while len(pending) >= PCM_CHUNK:
                    pcm = bytes(pending[:PCM_CHUNK])
                    del pending[:PCM_CHUNK]
                    # Pipe buffering can deliver many decoded blocks together.
                    # Pace PCM at its sample clock so the USB queue is not
                    # flooded by a decoder burst every few hundred ms.
                    now = time.monotonic()
                    if playback_origin is None or now - (playback_origin + self.samples / 44100) > .3:
                        playback_origin = now - self.samples / 44100
                    if self.closed.wait(max(0, playback_origin + self.samples / 44100 - now)):
                        return
                    self.emit(pcm, (self.origin or 0) + self.samples * 1000000 // 44100)
                    self.samples += PCM_CHUNK // 4
        except (OSError, ValueError):
            pass
        finally:
            if not self.closed.is_set():
                self.failed = True

    def close(self):
        self.closed.set()
        self.process.terminate()
        try:
            self.process.wait(timeout=.5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=.5)
        for t in self.threads:
            t.join(.5)
        self.process.stdin.close()
        self.process.stdout.close()


def run(args):
    import probe
    server = probe.listener(args.unix)
    link = None
    decoders, formats, retry_due = {}, {}, {}
    generation = [0]
    try:
        stream, _ = server.accept()
        source_clock = os.environ.get('CARLINK_LOW_LATENCY') == '1'
        capture_offset = None
        last_video_seq = None
        report_at = time.monotonic() + 5
        with stream:
            stream.settimeout(5)
            hello = probe.receive(stream)
            if not isinstance(hello, dict) or hello.get('type') != 'hello' or hello.get('role') != 'producer' or hello.get('version') != 0:
                raise ValueError('invalid producer hello')
            control_lock = threading.Lock()

            def feedback(value):
                try:
                    with control_lock:
                        probe.control(stream, value)
                except (OSError, ValueError, TypeError, KeyError):
                    pass

            probe.send_json(stream, dict(type='hello', version=0, role='consumer', name='CarLink live', client='carlink-live'))
            probe.send_json(stream, dict(type='capabilities', latency='INTERACTIVE',
                decoders=[dict(codecString='avc1.64001F', hardware=False)],
                displays={'0': dict(widthPx=args.width, heightPx=args.height, densityDpi=160,
                    refreshHz=30, frameRate='FPS_30', safeArea=dict(left=0, top=0, right=0, bottom=0))},
                touch=10, keys=list(range(10)), audio=[], microphone=False, prefersH264=True))
            link = LiveLink(args.live_socket, feedback)
            link.start()
            parameters, video_format, active = b'', None, False
            while True:
                if time.monotonic() >= report_at:
                    link.report()
                    report_at = time.monotonic() + 5
                ready, _, _ = select.select([stream], [], [], 1)
                if not ready:
                    continue
                message = probe.receive(stream)
                if isinstance(message, dict):
                    kind = message.get('type')
                    if kind == 'stats' and source_clock and message.get('clockOffsetUs') is not None:
                        value = message.get('clockOffsetUs')
                        if not isinstance(value, str) or not value.lstrip('-').isdigit() or not -(1 << 63) <= int(value) < (1 << 63):
                            raise ValueError('invalid producer clock mapping')
                        capture_offset = int(value)
                    elif kind == 'session':
                        next_active = message.get('session', {}).get('phase') == 'ACTIVE'
                        if next_active == active:
                            continue
                        active = next_active
                        generation[0] += 1
                        for decoder in decoders.values():
                            decoder.close()
                        decoders.clear()
                        formats.clear()
                        retry_due.clear()
                        link.session(active)
                        if active:
                            feedback({'type': 'keyframe', 'screen': 0})
                        else:
                            parameters, video_format = b'', None
                    elif kind == 'video-format' and message.get('screen') == 0:
                        fmt = message.get('format')
                        if fmt == video_format:
                            continue
                        video_format = fmt
                        if fmt is None:
                            parameters = b''
                            continue
                        if fmt.get('codec') != 'H264':
                            raise ValueError('live bridge requires H264')
                        parameters = probe.avc_parameters(base64.b64decode(fmt['config'], validate=True))
                        link.configure(fmt['width'], fmt['height'], 30)
                        feedback({'type': 'keyframe', 'screen': 0})
                    elif kind == 'audio-format':
                        ident, fmt = message.get('stream'), message.get('format')
                        old = formats.pop(ident, None)
                        if ident in decoders:
                            decoders.pop(ident).close()
                        if old and old.get('role') in ROLES:
                            link.audio_state(ROLES[old['role']], False)
                        if fmt is not None:
                            formats[ident] = fmt
                else:
                    kind, ident, flags, seq, pts, sender, payload = message
                    if not active:
                        continue
                    if kind == 1 and ident == 0 and parameters:
                        link.video_frames += 1
                        link.video_bytes += len(payload)
                        if last_video_seq is not None and seq != (last_video_seq + 1) & 0xffffffff and not flags & 1:
                            link.recover_video()
                        last_video_seq = seq
                        if source_clock:
                            age = (time.monotonic_ns() // 1000 - sender) / 1000000
                            link.source_ms_max = max(link.source_ms_max, age * 1000)
                            if age > VIDEO_BUDGET or age < -.05:
                                link.source_dropped += 1
                                link.recover_video()
                                continue
                            if capture_offset is None:
                                capture_offset = sender - pts
                        data = probe.annexb(payload)
                        if flags & 1:
                            data = parameters + data
                        if len(data) > MAX_VIDEO:
                            link.recover_video()
                            continue
                        link.media(2, data, sender if source_clock else pts, (flags & 1) | (2 if source_clock else 0))
                    elif kind == 2 and ident in formats:
                        fmt = formats[ident]
                        if fmt.get('role') not in ROLES or fmt.get('codec') not in ('AAC_LC', 'PCM_S16LE', 'OPUS'):
                            continue  # Phone/Siri microphone paths have separate protocol requirements.
                        role = ROLES[fmt['role']]
                        decoder = decoders.get(ident)
                        if decoder is not None and decoder.failed:
                            decoder.close()
                            del decoders[ident]
                            decoder = None
                            retry_due[ident] = time.monotonic() + .5
                        if decoder is None:
                            if time.monotonic() < retry_due.get(ident, 0):
                                continue
                            epoch = generation[0]

                            def emit(pcm, timestamp, r=role, e=epoch):
                                if generation[0] == e:
                                    link.media(3, pcm, timestamp, r)

                            decoder = AudioDecoder(args.audio_binary, fmt, emit)
                            decoders[ident] = decoder
                            link.audio_state(role, True)
                        decoder.feed(payload, pts + capture_offset if source_clock and capture_offset is not None else sender if source_clock else pts)
    finally:
        generation[0] += 1
        for decoder in decoders.values():
            decoder.close()
        if link:
            link.session(False)
            link.close()
        server.close()
        Path(args.unix).unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--unix', default='/run/carlink-carplay/media.sock')
    parser.add_argument('--live-socket', default=os.environ.get('CARLINK_LIVE_SOCKET', '/run/carlink/live.sock'))
    parser.add_argument('--audio-binary', default=str(Path(__file__).resolve().parent / 'bin/carlink-audio'))
    parser.add_argument('--width', type=int, default=800)
    parser.add_argument('--height', type=int, default=480)
    args, _ = parser.parse_known_args()
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    try:
        run(args)
    except KeyboardInterrupt:
        pass
    except (OSError, EOFError, ValueError, TypeError, KeyError) as error:
        print(json.dumps({'event': 'live_bridge_error', 'type': type(error).__name__}), flush=True)
        raise SystemExit(1)


if __name__ == '__main__':
    main()
