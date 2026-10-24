import asyncio
import unittest
from bridge import Bridge
from cipher import Cipher


class Link:
    def __init__(self, path, feedback):
        self.frames, self.states = [], []
    def configure(self, *args): pass
    def session(self, active): pass
    def start(self): pass
    def media(self, *args): self.frames.append(args)
    def audio_state(self, *args): self.states.append(args)
    def report(self): pass
    def close(self): pass


class Decoder:
    def __init__(self, executable, fmt, emit):
        self.emit, self.failed, self.frames = emit, False, []
    def feed(self, data, pts):
        self.frames.append((data, pts))
        return True
    def close(self): pass


class BridgeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.bridge = Bridge('/unused', '/unused', lambda *a, **kw: None, Link, Decoder)

    async def asyncTearDown(self):
        self.bridge.close()

    async def test_real_idr_requires_parameter_sets(self):
        b = self.bridge
        b.video(96, 0, b'\0\0\1\x65IDR')
        self.assertEqual(b.link.frames, [])
        b.video(96, 0, b'\0\0\1\x67SPS\0\0\1\x68PPS')
        b.video(96, 0, b'\0\0\1\x65IDR')
        frame = b.link.frames[-1]
        self.assertEqual(frame[3], 1)
        self.assertTrue(frame[1].startswith(b'\0\0\0\1\x67SPS\0\0\0\1\x68PPS'))
        self.assertGreater(frame[2], 0)
        b.video(96, 0, b'\0\0\1\x41P')
        self.assertEqual(b.link.frames[-1][3], 0)

    async def test_audio_generation_discards_late_decoder_output(self):
        b = self.bridge
        frame = dict(stream=1, rate=44100, channels=2, data=b'ADTS')
        b.audio(frame, 1000, 1)
        old = b.decoder
        self.assertEqual(old.frames, [(b'ADTS', 1000)])
        b.audio(frame, 2000, 2)
        old.emit(b'old', 1000)
        self.assertEqual(b.link.frames, [])
        b.decoder.emit(b'new', 2000)
        self.assertEqual(b.link.frames[-1], (3, b'new', 2000, 0))
        b.audio_stop()
        old.emit(b'old', 1000)
        self.assertEqual(len(b.link.frames), 1)

    async def test_same_speaker_ssrc_change_keeps_decoder(self):
        frame = dict(stream=1003, rate=48000, channels=2, data=b'ADTS')
        self.bridge.audio(frame, 1000, 1)
        decoder = self.bridge.decoder
        self.bridge.audio(dict(frame, stream=66539), 2000, 1)
        self.assertIs(self.bridge.decoder, decoder)
        self.assertEqual(len(decoder.frames), 2)

    async def test_input_focus_and_normalized_touch_encrypted(self):
        packets = []
        class Writer:
            def write(self, data): packets.append(data)
            async def drain(self): pass
        task = asyncio.create_task(self.bridge.input(Writer(), bytes(16)))
        await asyncio.sleep(0)
        self.bridge._enqueue(dict(type='key-press', code=3))
        self.bridge._enqueue(dict(type='touch', contacts=[dict(x=.5, y=.5, down=True)]))
        self.bridge._enqueue(dict(type='touch', contacts=[dict(x=.6, y=.5, down=True)]))
        self.bridge._enqueue(dict(type='touch', contacts=[dict(x=.5, y=.5, down=False)]))
        for _ in range(200):
            if len(packets) == 5: break
            await asyncio.sleep(.001)
        task.cancel()
        await asyncio.gather(task, return_exceptions=True)
        decoder = Cipher(bytes(16))
        try:
            plain = [decoder.decrypt(p[4:])[6:] for p in packets]
        finally:
            decoder.close()
        self.assertEqual([p[0] for p in plain], [3, 4, 0, 2, 1])
        self.assertEqual(int.from_bytes(plain[0][4:6], 'big'), 21)
        self.assertEqual(plain[2][5:9], bytes.fromhex('019000f0'))

    async def test_input_overflow_releases_contact_and_is_bounded(self):
        for _ in range(33):
            self.bridge._enqueue(dict(type='key-press', code=2))
        self.assertEqual(self.bridge.controls.qsize(), 1)
        self.assertEqual(self.bridge.controls.get_nowait(), dict(type='touch', contacts=[]))


if __name__ == '__main__':
    unittest.main()
