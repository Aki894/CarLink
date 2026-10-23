import unittest
from network import Offer
from network_wire import Resource, abilities
from nearby_wire import Frame, ProtocolError, pack_tlv, pack_uint, read_tlvs


def request(caps):
    fields = b''.join(pack_uint(key) + pack_uint(value) for key, value in caps.items())
    return Frame(4, 0, b'', pack_tlv(1, pack_uint(3) + pack_tlv(3, fields)))


class NetworkTests(unittest.TestCase):
    def setUp(self):
        self.offer = Offer('HiCar-Test', '02:00:00:00:00:01', 'private-example-password')
        self.resource = Resource(self.offer)
        self.frame = request({1: 1, 2: 1, 3: 1, 4: 1, 5: 511})

    def test_offer_validation_and_secret_repr(self):
        self.assertNotIn(self.offer.password, repr(self.offer))
        self.assertIn(b'##version:3##4005##2##hicarApIp:', self.offer.credentials())
        for ssid, password in [('a\nb', '12345678'), ('a', 'short'), ('a', 'x##12345678')]:
            with self.assertRaises(ValueError):
                Offer(ssid, self.offer.bssid, password)

    def test_no_network_offer_before_authentication(self):
        self.assertIsNone(self.resource.respond(self.frame))
        self.assertFalse(self.resource.negotiated)

    def test_negotiate_then_supply_credentials(self):
        self.resource.authenticated = True
        reply = self.resource.respond(self.frame)
        self.assertEqual(read_tlvs(reply), [(2, pack_uint(3) + pack_uint(1)
                                                + pack_tlv(3, b'\x01' + pack_uint(5745)))])
        self.assertNotIn(self.offer.password.encode(), reply)
        join = Frame(33, 0, b'', pack_tlv(4, b'02:00:00:00:00:02'))
        self.assertEqual(read_tlvs(self.resource.respond(join)), [(3, self.offer.credentials())])
        self.assertTrue(self.resource.credentials_sent)

    def test_reject_unsupported_band_and_malformed_requests(self):
        self.resource.authenticated = True
        self.assertEqual(read_tlvs(self.resource.respond(request({2: 1, 4: 0, 5: 511}))),
                         [(2, pack_uint(3) + pack_uint(255))])
        self.assertFalse(self.resource.negotiated)
        for frame in [Frame(33, 0, b'', pack_tlv(4, b'02:00:00:00:00:02')),
                      Frame(4, 0, b'', pack_tlv(1, b''))]:
            with self.assertRaises(ProtocolError):
                self.resource.respond(frame)
        for data in [b'\x81', b'\x81\x81\x81\x80', b'\x81\x01']:
            with self.assertRaises(ProtocolError):
                abilities(data)

    def test_selection_starts_gc_mac_exchange_on_protocol_33(self):
        self.resource.authenticated = True
        self.resource.respond(self.frame)
        selected = pack_uint(3) + pack_uint(1) + pack_tlv(3, b'\x02' + pack_uint(5745))
        self.assertIsNone(self.resource.respond(Frame(4, 0, b'', pack_tlv(7, selected))))
        self.assertEqual(self.resource.take_pending(), [(33, pack_tlv(1, b''))])
        self.assertEqual(self.resource.take_pending(), [])
        response = self.resource.respond(Frame(33, 0, b'', pack_tlv(2, b'02:00:00:00:00:02')))
        self.assertEqual(read_tlvs(response), [(3, self.offer.credentials())])

    def test_dmsdp_offer_requires_ready_listener_and_known_business(self):
        self.resource.authenticated = True
        self.resource.respond(self.frame)
        data = pack_uint(3) + pack_uint(20) + pack_uint(5) + b'test1' + pack_uint(1)
        frame = Frame(4, 0, b'', pack_tlv(3, data))
        with self.assertRaises(ProtocolError):
            self.resource.respond(frame)
        self.resource.data_port = 4006
        self.assertEqual(read_tlvs(self.resource.respond(frame)),
                         [(4, pack_uint(0) + pack_uint(4006) + data)])
        bad = pack_uint(3) + pack_uint(21) + pack_uint(0) + pack_uint(1)
        with self.assertRaises(ProtocolError):
            self.resource.respond(Frame(4, 0, b'', pack_tlv(3, bad)))

