import unittest

from nearby_wire import (Decoder, Incomplete, MAX_PAYLOAD, ProtocolError, crc16,
                         pack_frame, pack_tlv, pack_uint, read_tlvs, read_uint)


class WireTests(unittest.TestCase):
    def test_reference_integer_layout(self):
        for value, encoded in ((0, '80'), (127, 'ff'), (128, '0081'),
                               (16384, '000081'), ((1 << 28) - 1, '7f7f7fff')):
            self.assertEqual(pack_uint(value).hex(), encoded)
            self.assertEqual(read_uint(bytes.fromhex(encoded)), (value, len(encoded) // 2))
        with self.assertRaises(Incomplete):
            read_uint(b'\x00')
        for encoded in (b'\x00' * 4, b'\x7f' * 4):
            with self.assertRaises(ProtocolError):
                read_uint(encoded)
        for value in (-1, 1 << 28):
            with self.assertRaises(ProtocolError):
                pack_uint(value)

    def test_crc_arc_check_value(self):
        self.assertEqual(crc16(b'123456789'), 0xbb3d)

    def test_every_split_and_coalesced_frames(self):
        first = pack_frame(0, pack_tlv(1, b'\x02') + pack_tlv(5, b'\x00'))
        second = pack_frame(9, b'x' * 130)
        stream = first + second
        for cut in range(len(stream) + 1):
            decoder = Decoder()
            frames = decoder.feed(stream[:cut]) + decoder.feed(stream[cut:])
            decoder.finish()
            self.assertEqual([f.kind for f in frames], [0, 9])
            self.assertEqual(read_tlvs(frames[0].payload), [(1, b'\x02'), (5, b'\x00')])

    def test_encrypted_payload_is_preserved(self):
        raw = b'\xff\x00\x7f'
        options = pack_tlv(1, b'12345678')
        frame, = Decoder().feed(pack_frame(9, raw, 1, options))
        self.assertTrue(frame.encrypted)
        self.assertEqual((frame.payload, frame.options), (raw, options))

    def test_corruption_fails_closed(self):
        good = pack_frame(0, b'hello')
        for index in range(len(good)):
            broken = bytearray(good)
            broken[index] ^= 1
            decoder = Decoder()
            with self.assertRaises(ProtocolError):
                decoder.feed(broken)
                decoder.finish()

    def test_hostile_lengths_and_truncation(self):
        for raw in (b'\x7f\x80\x80' + pack_uint(257),
                    b'\x7f\x80\x80\x80' + pack_uint(MAX_PAYLOAD + 1)):
            decoder = Decoder()
            with self.assertRaises(ProtocolError):
                decoder.feed(raw)
            self.assertEqual(len(decoder.buffer), 0)
            with self.assertRaises(ProtocolError):
                decoder.feed(pack_frame(0, b''))
        for raw in (b'\x81', b'\x81\x82x'):
            with self.assertRaises(ProtocolError):
                read_tlvs(raw)

    def test_max_payload_and_many_combined_frames(self):
        frame, = Decoder().feed(pack_frame(10, b'\x7f' * MAX_PAYLOAD))
        self.assertEqual(len(frame.payload), MAX_PAYLOAD)
        frames = Decoder().feed(pack_frame(0, b'') * 3000)
        self.assertEqual(len(frames), 3000)
