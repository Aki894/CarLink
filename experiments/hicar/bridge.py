"""Experimental HiCar source adapter for the existing bounded CarLife link."""
import asyncio
import importlib.util
import math
from pathlib import Path
import re
import subprocess
import time

from hisight_input import Input
from nearby_wire import ProtocolError


def shared_transport():
    path = Path(__file__).resolve().parents[2] / 'receiver' / 'forward.py'
    spec = importlib.util.spec_from_file_location('carlink_shared_forward', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


TRANSPORT = shared_transport()
KEYS = {0: 'home', 1: 'back', 2: 'enter', 3: 'left', 4: 'right',
        5: 'up', 6: 'down'}


def nal_units(data):
    return [n for n in re.split(b'\x00\x00\x00?\x01', data) if n]


class Bridge:
    """One authenticated control session; no disk capture or network ownership."""
    def __init__(self, path, audio_binary, log, link_factory=None, decoder_factory=None):
        self.loop = asyncio.get_running_loop()
        self.log, self.audio_binary = log, audio_binary
        self.decoder_factory = decoder_factory or TRANSPORT.AudioDecoder
        self.controls = asyncio.Queue(maxsize=32)
        self.closed = False
        self.decoder = self.audio_format = None
        self.audio_epoch = 0
        self.parameters = {}
        self.video_frames = self.audio_frames = self.input_events = 0
        self.last_report = time.monotonic()
        self.link = (link_factory or TRANSPORT.LiveLink)(path, self.feedback)
        self.link.configure(800, 480, 30)
        self.link.session(True)
        self.link.start()

    def feedback(self, value):
        if not self.closed:
            self.loop.call_soon_threadsafe(self._enqueue, value)

    def _enqueue(self, value):
        if self.closed or not isinstance(value, dict):
            return
        if value.get('type') == 'keyframe':
            # The source negotiates GOP 30. Cache parameter sets and wait for
            # its next real IDR; never label a predicted frame as a keyframe.
            return
        try:
            self.controls.put_nowait(value)
        except asyncio.QueueFull:
            # A lost release must not leave a contact stuck down.
            while not self.controls.empty():
                self.controls.get_nowait()
            self.controls.put_nowait({'type': 'touch', 'contacts': []})
            self.log('bridge_input_overflow')

    def video(self, kind, timestamp, data):
        if self.closed or kind != 96:
            return
        if len(data) > TRANSPORT.MAX_VIDEO - 1024:
            self.link.recover_video()
            self.log('bridge_video_rejected', reason='frame_too_large')
            return
        units = nal_units(data)
        for unit in units:
            if unit[0] & 31 in (7, 8):
                self.parameters[unit[0] & 31] = b'\0\0\0\1' + unit
        key = any(unit[0] & 31 == 5 for unit in units)
        if key:
            if set(self.parameters) != {7, 8}:
                return
            data = self.parameters[7] + self.parameters[8] + data
        # Video RTP timestamps use the peer clock. Until cross-stream video
        # clock mapping is verified, stamp local arrival instead of mixing
        # unrelated peer and PCM timelines in the CarLife output.
        self.link.media(2, data, time.monotonic_ns() // 1000, int(key))
        self.video_frames += 1
        self.report()

    def audio_stop(self):
        self.audio_epoch += 1
        decoder, self.decoder = self.decoder, None
        self.audio_format = None
        if decoder:
            decoder.close()
        self.link.audio_state(0, False)

    def audio(self, frame, pts, generation):
        if self.closed:
            return
        fmt = (generation, frame['stream'], frame['rate'], frame['channels'])
        if fmt != self.audio_format or self.decoder and self.decoder.failed:
            self.audio_stop()
            epoch = self.audio_epoch
            def emit(pcm, timestamp):
                if not self.closed and self.audio_epoch == epoch:
                    self.link.media(3, pcm, timestamp, 0)
            self.decoder = self.decoder_factory(self.audio_binary,
                dict(codec='AAC_ADTS', sampleRate=frame['rate'], channels=frame['channels']), emit)
            self.audio_format = fmt
            self.link.audio_state(0, True)
        if not self.decoder.feed(frame['data'], pts):
            self.audio_stop()
            self.log('bridge_audio_reset', reason='decoder_backpressure')
        self.audio_frames += 1
        self.report()

    def report(self):
        if time.monotonic() - self.last_report >= 5:
            self.link.report()
            self.log('bridge_stats', video_frames=self.video_frames,
                     audio_frames=self.audio_frames, input_events=self.input_events)
            self.last_report = time.monotonic()

    async def input(self, writer, key):
        encoder = Input(key)
        contact = None
        while not self.controls.empty():
            self.controls.get_nowait()
        try:
            while not self.closed:
                item = await self.controls.get()
                packets = []
                if item.get('type') == 'key-press' and type(item.get('code')) is int:
                    name = KEYS.get(item['code'])
                    if name:
                        packets = [encoder.key(name, True), encoder.key(name, False)]
                elif item.get('type') == 'touch':
                    contacts = item.get('contacts')
                    if not isinstance(contacts, list) or len(contacts) > 10:
                        continue
                    if not contacts:
                        if contact:
                            packets = [encoder.touch(*contact, False)]
                            contact = None
                    else:
                        point = contacts[0]
                        if not isinstance(point, dict):
                            continue
                        x, y, down = point.get('x'), point.get('y'), point.get('down')
                        if (type(x) not in (int, float) or type(y) not in (int, float)
                                or not math.isfinite(x) or not math.isfinite(y)
                                or not 0 <= x <= 1 or not 0 <= y <= 1 or type(down) is not bool):
                            continue
                        xy = (min(799, int(x * 800)), min(479, int(y * 480)))
                        packets = [encoder.touch(*xy, down)]
                        contact = xy if down else None
                for packet in packets:
                    writer.write(packet)
                    async with asyncio.timeout(1):
                        await writer.drain()
                if packets:
                    self.input_events += 1
                    self.log('bridge_input_sent', kind=item['type'])
        except (OSError, TimeoutError, ProtocolError) as error:
            self.log('bridge_input_stopped', error=type(error).__name__)
        finally:
            try:
                if contact:
                    writer.write(encoder.touch(*contact, False))
                    async with asyncio.timeout(.2):
                        await writer.drain()
            except (OSError, TimeoutError):
                pass
            encoder.cipher.close()

    def close(self):
        if not self.closed:
            self.closed = True
            self.audio_stop()
            self.link.session(False)
            self.link.close()
            self.log('bridge_closed', video_frames=self.video_frames,
                     audio_frames=self.audio_frames, input_events=self.input_events)


class CarLife:
    """Temporary output process, started after the experiment owns the radio."""
    def __init__(self, executable, path):
        self.executable, self.path = executable, path

    def __enter__(self):
        path = Path(self.path)
        if not path.is_absolute() or path.exists() or path.is_symlink():
            raise ValueError('live socket must be an unused absolute path')
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        if path.parent.stat().st_mode & 0o077:
            raise ValueError('live socket parent must be private')
        self.process = subprocess.Popen([self.executable, 'carlife', 'usb',
            '--duration', '0', '--live-socket', self.path])
        return self

    def __exit__(self, *_):
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=2)
        Path(self.path).unlink(missing_ok=True)
