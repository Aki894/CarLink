import unittest
from dmsdp_wire import Decoder, Negotiation, Packet, pack
from nearby_wire import ProtocolError, pack_uint


class DmsdpTests(unittest.TestCase):
    def test_fragmentation_and_combined_packets(self):
        wire = pack(1, bytes(16)) + pack(2, b'fragment')
        for split in range(len(wire) + 1):
            decoder = Decoder()
            self.assertEqual(decoder.feed(wire[:split]) + decoder.feed(wire[split:]),
                             [Packet(1, bytes(16)), Packet(2, b'fragment')])
            decoder.finish()

    def test_control_port_negotiation(self):
        negotiation = Negotiation(4007)
        packet = Decoder().feed(negotiation.respond(Packet(1, bytes(16))))[0]
        self.assertEqual(packet.module, 1)
        self.assertEqual(packet.payload, b'\x00\x0f\xa7' + bytes(13))
        self.assertTrue(negotiation.complete)
        for invalid in [Packet(2, bytes(16)), Packet(1, bytes(15)), Packet(1, b'\x01' + bytes(15))]:
            with self.assertRaises(ProtocolError):
                Negotiation(4007).respond(invalid)

    def test_corruption_length_and_incomplete_packets(self):
        corrupt = bytearray(pack(1, bytes(16)))
        corrupt[-1] ^= 1
        for wire in [b'bad', bytes(corrupt), b'\x7f\x81' + pack_uint(65537)]:
            decoder = Decoder()
            with self.assertRaises(ProtocolError):
                decoder.feed(wire)
            with self.assertRaises(ProtocolError):
                decoder.feed(pack(1, b'ok'))
        decoder = Decoder()
        decoder.feed(pack(1, bytes(16))[:-1])
        with self.assertRaises(ProtocolError):
            decoder.finish()
