import struct
import unittest
from unittest.mock import patch
import voice


class MicrophoneTests(unittest.TestCase):
    def setUp(self):
        self.sent, self.requests = [], []
        self.mic = voice.Microphone(self.sent.append, self.requests.append)
        self.mic.generation = 0

    def config(self, rate=16000, stream=1):
        self.mic.configure(stream, dict(codec='PCM_S16LE', sampleRate=rate, channels=1, frameMs=20))

    def test_exact_pcm_framing_and_partial_input(self):
        self.config()
        data = struct.pack('<h', 1234) * 320
        self.mic.feed(1, data[:200])
        self.assertFalse(self.sent)
        self.mic.feed(1, data[200:])
        self.assertEqual(self.sent[0][28:], data)
        header = voice.HEADER.unpack_from(self.sent[0], 4)
        self.assertEqual(header[:5], (3, 1, 0, 0, 0))

    def test_generation_stop_and_stream_fencing(self):
        self.config(); self.config(stream=2)
        self.mic.feed(1, bytes(640))
        self.assertFalse(self.sent)
        self.mic.stop(1)
        self.assertIsNotNone(self.mic.current)
        self.mic.stop(2)
        self.mic.feed(2, bytes(640))
        self.assertFalse(self.sent)
        self.assertEqual([r['active'] for r in self.requests], [True, False, True, False])

    def test_resampling_across_irregular_chunks(self):
        for rate in (8000, 24000, 32000, 44100, 48000):
            self.sent.clear(); self.config(rate)
            for _ in range(50):
                self.mic.feed(self.mic.generation, struct.pack('<h', -1234) * 337)
            self.assertGreater(len(self.sent), 45)
            for packet in self.sent:
                self.assertEqual(len(packet[28:]), rate // 50 * 2)
                self.assertEqual(packet[28:], struct.pack('<h', -1234) * (rate // 50))
            self.assertLess(len(self.mic.pcm), rate // 50 * 2)
            self.assertLessEqual(len(self.mic.samples), 2)

    def test_reject_formats_and_malformed_pcm(self):
        with self.assertRaises(ValueError):
            self.mic.configure(1, dict(codec='OPUS', sampleRate=16000, channels=1, frameMs=20))
        self.config()
        for data in (b'', b'x', bytes(16386)):
            with self.assertRaises(ValueError): self.mic.feed(1, data)

    def test_stop_clears_partial_audio(self):
        self.config(); self.mic.feed(1, bytes(638)); self.mic.stop()
        self.config(); self.mic.feed(2, bytes(2)); self.assertFalse(self.sent)

    def test_opus_uses_48k_capture_and_fences_encoder(self):
        with patch('opus.Encoder') as encoder:
            encoder.return_value.encode.return_value = b'\xf8\xff\xfe'
            self.mic.configure(1, dict(codec='OPUS', sampleRate=24000, channels=2, frameMs=20))
            for _ in range(2):
                self.mic.feed(self.mic.generation, struct.pack('<h', 1234) * 320)
            encoder.assert_called_once_with(2)
            self.assertEqual(len(self.sent), 1)
            pcm = encoder.return_value.encode.call_args.args[0]
            self.assertEqual(len(pcm), 1920)
            self.assertEqual(pcm, struct.pack('<h', 1234) * 960)
            self.assertEqual(self.sent[0][28:], b'\xf8\xff\xfe')
            self.mic.stop()
            encoder.return_value.close.assert_called_once()
            self.assertIsNone(self.mic.encoder)


if __name__ == '__main__':
    unittest.main()
