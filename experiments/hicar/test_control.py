import unittest
from control import Negotiator
from nearby_wire import Decoder, ProtocolError, pack_frame, pack_tlv, read_tlvs


class ControlTests(unittest.TestCase):
    def frame(self, payload, kind=0, flags=0):
        return Decoder().feed(pack_frame(kind, payload, flags))[0]

    def test_initial_exchange_without_authentication(self):
        state = Negotiator(b'local-summary')
        request = pack_tlv(1, b'\x02') + pack_tlv(3, b'peer-summary') + pack_tlv(5, b'\x01')
        response = state.respond(self.frame(request))
        self.assertEqual(read_tlvs(response), [(2, b'\x02'), (4, b'local-summary'), (6, b'\x00')])
        self.assertTrue(state.complete)

    def test_multiple_messages_and_no_response_loop(self):
        state = Negotiator(b'local-summary')
        for tag, value in ((1, b'\x02'), (3, b'peer-summary'), (5, b'\x00')):
            self.assertIsNotNone(state.respond(self.frame(pack_tlv(tag, value))))
        self.assertTrue(state.complete)
        self.assertIsNone(state.respond(self.frame(pack_tlv(2, b'\x02'))))
        self.assertIsNone(state.respond(self.frame(b'business', kind=9)))
        self.assertIsNone(state.respond(self.frame(b'encrypted', flags=1)))

    def test_phone_v5_offer_can_select_legacy_v2(self):
        state = Negotiator(b'local-summary')
        request = pack_tlv(1, b'\x05') + pack_tlv(3, b'001122334455') + pack_tlv(5, b'\x02')
        self.assertEqual(read_tlvs(state.respond(self.frame(request))),
                         [(2, b'\x02'), (4, b'local-summary'), (6, b'\x00')])
        self.assertTrue(state.complete)

    def test_invalid_request_does_not_commit_partial_state(self):
        state = Negotiator(b'local-summary')
        request = pack_tlv(1, b'\x02') + pack_tlv(5, b'\x99')
        with self.assertRaises(ProtocolError):
            state.respond(self.frame(request))
        self.assertEqual(state.seen, set())

    def test_business_endpoint_only_after_control_and_listener_ready(self):
        from nearby_wire import pack_uint
        body = b''.join(map(pack_uint, (2, 19, 8))) + b'test-tag' + pack_uint(0)
        request = self.frame(pack_tlv(3, body), kind=2)
        state = Negotiator(b'local-summary', business_channel=24)
        self.assertIsNone(state.respond(request))
        state.respond(self.frame(pack_tlv(1, b'\x05') + pack_tlv(3, b'peer') + pack_tlv(5, b'\x02')))
        response = read_tlvs(state.respond(request))
        self.assertEqual(response[0][0], 4)
        self.assertEqual(response[0][1][:2], pack_uint(0) + pack_uint(24))
        self.assertEqual(response[0][1][2:2+len(body)], body)
        bad = b''.join(map(pack_uint, (2, 20, 8))) + b'test-tag' + pack_uint(0)
        with self.assertRaises(ProtocolError):
            state.respond(self.frame(pack_tlv(3, bad), kind=2))
