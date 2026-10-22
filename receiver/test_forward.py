import struct
import unittest
from unittest.mock import patch

import forward


class ForwardTests(unittest.TestCase):
    def test_live_header_matches_rust_protocol(self):
        data = forward.packet(3, b'\0' * 4, 1, 23456)
        self.assertEqual(struct.unpack_from('<I', data)[0], 16)
        self.assertEqual(forward.LIVE.unpack_from(data, 4), (3, 1, 0, 23456))
        self.assertEqual(data[16:], b'\0' * 4)
        with self.assertRaises(ValueError):
            forward.packet(2, b'x' * (forward.MAX_VIDEO + 1))

    def test_adts_length_and_format(self):
        data = forward.adts(b'abc', 48000, 2)
        self.assertEqual(data[:3], bytes([255, 241, 76]))
        length = ((data[3] & 3) << 11) | (data[4] << 3) | (data[5] >> 5)
        self.assertEqual(length, len(data))
        self.assertEqual((data[3] >> 6) | ((data[2] & 1) << 2), 2)
        for payload, rate, channels in [(b'', 48000, 2), (b'x' * 8185, 48000, 2), (b'x', 123, 2)]:
            with self.assertRaises(ValueError):
                forward.adts(payload, rate, channels)

    def test_ogg_opus_packet_headers_and_duration(self):
        head = b'OpusHead' + struct.pack('<BBHIhB', 1, 1, 0, 48000, 0, 0)
        page = forward.ogg_page(head, 0, 0, True)
        self.assertEqual(page[:6], b'OggS\x00\x02')
        self.assertEqual(page[26], 1)
        self.assertEqual(page[27], len(head))
        self.assertEqual(page[28:], head)
        self.assertNotEqual(page[22:26], b'\0' * 4)
        self.assertEqual(forward.opus_samples(b'\xf8\xff\xfe'), 960)
        with self.assertRaises(ValueError):
            forward.opus_samples(b'\x03')

    def test_overload_waits_for_new_keyframe(self):
        controls = []
        link = forward.LiveLink('unused', controls.append)
        link.connected = True
        link.session(True)
        link.configure(800, 480, 30)
        self.assertFalse(link.media(2, b'P', 1))
        self.assertTrue(link.media(2, b'I', 2, 1))
        self.assertTrue(link.media(2, b'P', 3))
        link.size = forward.MAX_QUEUE
        self.assertFalse(link.media(2, b'P', 4))
        self.assertTrue(link.wait_key)
        self.assertEqual(controls[-1]['type'], 'keyframe')
        self.assertFalse(link.media(2, b'P', 5))
        self.assertTrue(link.media(2, b'I', 6, 1))

    def test_audio_queue_is_bounded_and_stop_clears_only_its_role(self):
        link = forward.LiveLink('unused', lambda _: None)
        link.connected = True
        link.session(True)
        link.configure(800, 480, 30)
        for _ in range(10):
            self.assertTrue(link.media(3, b'\0' * 4, 0))
        self.assertFalse(link.media(3, b'\0' * 4, 0))
        link.audio_state(0, False)
        self.assertFalse(any(k == 3 for k, _, _ in link.pending))
        self.assertEqual(link.pending[-1][1][4:6], b'\x05\x00')


if __name__ == '__main__':
    unittest.main()
