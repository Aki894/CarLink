"""Generation-fenced CarLife PCM16 microphone to OXP PCM uplink. No recording."""
import array
import json
import struct
import sys
import threading
import time

HEADER = struct.Struct('<BBBBIqq')


class Microphone:
    def __init__(self, send, request):
        self.send, self.request = send, request
        self.lock = threading.RLock()
        self.current = None
        self.generation = time.monotonic_ns()

    def stop(self, stream=None):
        with self.lock:
            if self.current is None or stream is not None and self.current['stream'] != stream:
                return
            self.request(dict(generation=self.generation, active=False, sample_rate=16000, channels=1))
            self.current = None
            self.samples, self.pcm, self.position = [], bytearray(), 0

    def configure(self, stream, request):
        if type(stream) is not int or not 0 <= stream <= 255:
            raise ValueError('invalid microphone stream')
        with self.lock:
            if request is None:
                self.stop(stream)
                return
            if (not isinstance(request, dict) or request.get('codec') != 'PCM_S16LE'
                    or type(request.get('sampleRate')) is not int
                    or request['sampleRate'] not in (8000, 16000, 24000, 32000, 44100, 48000)
                    or request.get('channels') != 1 or request.get('frameMs') != 20):
                raise ValueError('unsupported microphone format')
            self.stop()
            self.generation += 1
            self.current = dict(request, stream=stream)
            self.samples, self.pcm, self.position = [], bytearray(), 0
            self.sequence, self.sent_samples = 0, 0
            self.origin = time.monotonic_ns() // 1000
            self.request(dict(generation=self.generation, active=True, sample_rate=16000, channels=1))

    def feed(self, generation, data):
        with self.lock:
            if self.current is None or generation != self.generation:
                return
            if not data or len(data) > 16384 or len(data) % 2:
                raise ValueError('invalid microphone PCM')
            values = array.array('h', data)
            if sys.byteorder != 'little':
                values.byteswap()
            rate = self.current['sampleRate']
            if rate == 16000:
                self.pcm.extend(data)
            else:
                self.samples.extend(values)
                out = array.array('h')
                while self.position // rate + 1 < len(self.samples):
                    index, fraction = divmod(self.position, rate)
                    out.append((self.samples[index] * (rate - fraction) + self.samples[index + 1] * fraction) // rate)
                    self.position += 16000
                consumed = min(self.position // rate, len(self.samples) - 1)
                self.samples = self.samples[consumed:]
                self.position -= consumed * rate
                if sys.byteorder != 'little':
                    out.byteswap()
                self.pcm.extend(out.tobytes())
            size = rate // 50 * 2
            while len(self.pcm) >= size:
                payload = bytes(self.pcm[:size])
                del self.pcm[:size]
                pts = self.origin + self.sent_samples * 1000000 // rate
                body = HEADER.pack(3, self.current['stream'], 0, 0, self.sequence, pts,
                                   time.monotonic_ns() // 1000) + payload
                self.send(struct.pack('<I', len(body)) + body)
                self.sequence = (self.sequence + 1) & 0xffffffff
                self.sent_samples += size // 2


def request_bytes(value):
    return json.dumps(value, separators=(',', ':')).encode()
